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

use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CanonicalId, ClockProviderFailure, ClockReadError, ClockRef, CmdError,
    CommandSpec, Ctx, Engine, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind, OwnerId,
    World,
};
use mush_economy::{
    AccountId, AccountRevision, AccountStatus, Amount, CADENCE_SCHEMA_DECLARED, CadenceId,
    CadenceSource, Clocks, ContractError, ContractId, ContractState, CustodyError, CustodyProvider,
    CustodyRef, DeclaredCadence, DeclaredCadenceClock, DeclaredCadenceError, EconomicCause,
    EconomicClock, EconomicSchedule, EconomyConfig, EconomyError, EconomyPlan, EconomySnapshot,
    EconomyStore, Encumbrance, EscalationKind, EscalationSubject, ForgiveCause, GracePolicy,
    HoldId, HoldStatus, IdempotencyKey, InstrumentCaps, LeaseEnd, ObligationCause, ObligationEvent,
    ObligationId, ObligationSpec, ObligationStatus, PostOutcome, RequestedLine, SuspendReason,
    SuspensionKey,
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
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
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
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    store(tw).snapshot()
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn held(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").held_amount.minor()
}
fn journal_len(tw: &TestWorld) -> usize {
    store(tw).transactions().count()
}
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(LIMBO))
        .expect("create")
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
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
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
    kv.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("no {key}"))
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
        ok(
            tw,
            GOD,
            &format!("@econ/mint {faucet}={pat_acct} {pat_funds}"),
        );
    }
    Rig {
        pat,
        quinn,
        pat_acct,
        quinn_acct,
        faucet,
    }
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

    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 deposit", r.pat_acct),
    );
    assert_eq!(
        held(&tw, r.pat_acct),
        60,
        "the reservation is not projected"
    );
    assert_eq!(bal(&tw, r.pat_acct), 100, "a reservation moved money");

    let seen = err(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 second", r.pat_acct),
    );
    assert!(
        seen.contains("insufficient available"),
        "wrong refusal: {seen}"
    );
    assert_eq!(
        held(&tw, r.pat_acct),
        60,
        "the refused placement still reserved"
    );
    assert_eq!(
        snap(&tw).holds().count(),
        1,
        "the refused placement minted a hold"
    );
}

#[test]
fn a_payment_that_would_dip_into_held_funds_is_refused() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 deposit", r.pat_acct),
    );
    let before = journal_len(&tw);

    let seen = err(
        &mut tw,
        r.pat,
        &format!("@econ/pay {}={} 50", r.pat_acct, r.quinn_acct),
    );
    assert!(
        seen.contains("insufficient available"),
        "wrong refusal: {seen}"
    );
    assert_eq!(bal(&tw, r.pat_acct), 100, "the refused payment moved money");
    assert_eq!(bal(&tw, r.quinn_acct), 0);
    assert_eq!(
        journal_len(&tw),
        before,
        "the refused payment journaled something"
    );

    // Exactly the uncommitted part still spends.
    ok(
        &mut tw,
        r.pat,
        &format!("@econ/pay {}={} 40", r.pat_acct, r.quinn_acct),
    );
    assert_eq!(bal(&tw, r.pat_acct), 60);
    assert_eq!(held(&tw, r.pat_acct), 60);
}

#[test]
fn the_beneficiary_captures_and_the_owners_own_wallet_rights_do_not() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 for=#{} escrow", r.pat_acct, r.quinn.0),
    );
    let hold = only_hold(&tw);

    // The payer owns the account and may spend it — and still cannot spend the
    // reservation standing between them and their own money (the E4a fold, finding 1).
    err(
        &mut tw,
        r.pat,
        &format!("@hold/capture {hold}={} 60", r.pat_acct),
    );
    assert_eq!(bal(&tw, r.quinn_acct), 0);

    ok(
        &mut tw,
        r.quinn,
        &format!("@hold/capture {hold}={} 40", r.quinn_acct),
    );
    assert_eq!(
        bal(&tw, r.pat_acct),
        60,
        "the capture debited the wrong amount"
    );
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
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 for=#{} escrow", r.pat_acct, r.quinn.0),
    );
    let hold = only_hold(&tw);
    ok(
        &mut tw,
        r.quinn,
        &format!("@hold/capture {hold}={} 60", r.quinn_acct),
    );
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
    assert_eq!(
        bal(&tw, r.quinn_acct),
        60,
        "the refused reversal moved money"
    );
}

#[test]
fn account_close_is_refused_while_a_hold_or_an_obligation_lives() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=100 deposit", r.pat_acct),
    );
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
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {}={} 100", r.pat_acct, r.quinn_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100",
            r.pat_acct, r.quinn_acct
        ),
    );
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
        ok(
            &mut tw,
            GOD,
            &format!(
                "@hold/place {}=60 for=#{} expires=10 a",
                r.pat_acct, r.quinn.0
            ),
        );
        ok(
            &mut tw,
            GOD,
            &format!("@hold/place {}=90 for=#{} b", r.pat_acct, r.quinn.0),
        );
        // The hold allocator's snapshot floor starts at 1 (E4a `next_hold_floor`), so the
        // first reservation in a fresh economy is #1.
        let first = HoldId(1);
        match act {
            "capture" => ok(
                &mut tw,
                r.quinn,
                &format!("@hold/capture {first}={} 25", r.quinn_acct),
            ),
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

        assert!(
            !store(&tw).is_frozen(),
            "{act}: a clean reboot froze the economy"
        );
        assert_eq!(
            held(&tw, r.pat_acct),
            before_held,
            "{act}: held_amount changed across reboot"
        );
        assert_eq!(
            bal(&tw, r.pat_acct),
            before_bal,
            "{act}: balance changed across reboot"
        );
        assert_eq!(
            snap(&tw).hold(first).expect("hold").status,
            before_status,
            "{act}: hold state changed across reboot"
        );
        assert_eq!(
            held(&tw, r.pat_acct),
            90,
            "{act}: the second reservation was lost"
        );
    }
}

#[test]
fn firing_the_sweep_twice_at_one_instant_expires_a_hold_once() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 expires=10 a", r.pat_acct),
    );
    let hold = only_hold(&tw);
    let before = journal_len(&tw);

    tw.set_now(100);
    tw.fire_due(100_000);
    // The SAME instant, a second scheduler delivery — the exit criterion's duplicate
    // scheduler event. The deterministic key `hold/{id}/expire` resolves it.
    tw.fire_due(130_000);

    assert_eq!(
        journal_len(&tw),
        before + 1,
        "the duplicate fire posted a second expiry"
    );
    assert_eq!(
        snap(&tw).hold(hold).expect("hold").status,
        HoldStatus::Expired
    );
    assert_eq!(
        held(&tw, r.pat_acct),
        0,
        "expiry did not free the reservation"
    );
    assert_eq!(bal(&tw, r.pat_acct), 100, "expiry moved money");
}

#[test]
fn a_hold_on_a_manual_clock_is_never_aged_out_by_the_sweep() {
    // A `Manual` deadline resolves to "never passed" for the sweep: a manually-clocked
    // reservation is released by hand, never by a scheduler reading someone else's
    // clock. Built through the store because no command offers a manual deadline.
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 expires=ic:1000000 a", r.pat_acct),
    );
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

    assert!(
        !store(&tw).is_frozen(),
        "a lagging projection froze the economy"
    );
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

    assert!(
        store(&tw).is_frozen(),
        "drift at the claimed sequence did not freeze"
    );
    // Healed to the journal, and every account frozen with it.
    assert_eq!(snap(&tw).hold(HoldId(1)).expect("hold").amount.minor(), 60);
    assert_eq!(
        snap(&tw).account(r.pat_acct).expect("account").status,
        AccountStatus::Frozen
    );
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

    assert!(
        !store(&tw).is_frozen(),
        "a lost projection froze the economy"
    );
    assert_eq!(
        snap(&tw)
            .hold(HoldId(1))
            .expect("the hold was not rebuilt")
            .amount
            .minor(),
        60,
        "a missing hold doc must rebuild from the journal"
    );
    assert!(
        snap(&tw).hold(HoldId(77)).is_none(),
        "an unjournaled hold doc was loaded"
    );
    assert_eq!(
        held(&tw, r.pat_acct),
        60,
        "the forged doc reached held_amount"
    );
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
    assert!(
        store(&tw).is_frozen(),
        "held_amount drift at last_hold_seq did not freeze"
    );
    assert_eq!(
        held(&tw, r.pat_acct),
        60,
        "the drifted projection was not healed"
    );
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
    assert_eq!(
        held(&tw, r.pat_acct),
        80,
        "the lagging projection was not healed forward"
    );
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
    assert!(
        snap(&tw).hold(HoldId(2)).is_none(),
        "a terminal hold was not collected"
    );
    assert_eq!(
        store(&tw).peek_next_hold(),
        3,
        "the allocator floor dropped after GC"
    );

    // And the floor is journal-derived, so it survives the reboot that loses the doc.
    let kv = fold_kv(&mut tw);
    assert!(
        !kv.iter().any(|(k, _)| k == "hold:2"),
        "the collected doc was not deleted"
    );
    reboot(&mut tw, kv);
    assert_eq!(
        store(&tw).peek_next_hold(),
        3,
        "the reboot floor came from the docs"
    );

    ok(&mut tw, GOD, &format!("@hold/place {}=10 c", r.pat_acct));
    assert!(
        snap(&tw).hold(HoldId(3)).is_some(),
        "the new reservation re-minted a collected id"
    );
}

// ======================================================================
// Hidden parties (§4.5)
// ======================================================================

#[test]
fn a_third_party_cannot_tell_a_hold_or_an_obligation_from_a_nonexistent_one() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    let nosy = tw.spawn_player("Nosy");
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 for=#{} escrow", r.pat_acct, r.quinn.0),
    );
    let hold = only_hold(&tw);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100",
            r.pat_acct, r.quinn_acct
        ),
    );
    let obligation_id = only_obligation(&tw);

    let a = err(&mut tw, nosy, &format!("@hold/show {hold}"));
    let b = err(&mut tw, nosy, "@hold/show 4242");
    assert_eq!(
        a, b,
        "an existing hold reads differently from a nonexistent one"
    );
    let a = err(&mut tw, nosy, &format!("@hold/release {hold}"));
    assert_eq!(a, b, "a refusal to release disclosed the hold");
    let a = err(&mut tw, nosy, &format!("@obligation/show {obligation_id}"));
    let b = err(&mut tw, nosy, "@obligation/show 4242");
    assert_eq!(
        a, b,
        "an existing obligation reads differently from a nonexistent one"
    );

    // The list is empty rather than a count of what is hidden.
    let seen = ok(&mut tw, nosy, "@hold/list");
    assert!(seen.contains("No holds."), "the list leaked: {seen}");
    let seen = ok(&mut tw, nosy, "@obligation/list");
    assert!(seen.contains("No obligations."), "the list leaked: {seen}");

    // `@hold/list account=` follows E3's list discipline: identical text for a
    // nonexistent account and one the actor may not view.
    let a = err(&mut tw, nosy, &format!("@hold/list account={}", r.pat_acct));
    let b = err(&mut tw, nosy, "@hold/list account=4242");
    assert_eq!(
        a, b,
        "an account the actor may not view read differently from a missing one"
    );
    // ...and empty, not an error, for a viewable account with no reservations.
    let seen = ok(
        &mut tw,
        r.quinn,
        &format!("@hold/list account={}", r.quinn_acct),
    );
    assert!(
        seen.contains("No holds."),
        "a viewable empty account errored: {seen}"
    );

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
    assert_eq!(
        bal(&tw, r.pat_acct),
        800,
        "the second period did not settle"
    );
    assert!(
        !texts(&ev).contains("Obligation"),
        "routine settlement spoke"
    );

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

    assert_eq!(
        journal_len(&tw),
        before + 1,
        "the duplicate fire posted twice"
    );
    assert_eq!(
        bal(&tw, r.pat_acct),
        900,
        "the duplicate fire charged twice"
    );
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
    assert!(matches!(
        obligation(&tw, id).status,
        ObligationStatus::Grace { missed: 1, .. }
    ));
    let said = texts(&ev);
    assert!(said.contains("missed a period"), "grace was silent: {said}");
    assert!(
        !said.contains("durab"),
        "a sweep notice claimed durability: {said}"
    );
    assert_eq!(bal(&tw, r.pat_acct), 50, "a missed period moved money");

    let ev = sweep_at(&mut tw, 300);
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);
    assert!(texts(&ev).contains("delinquent"), "delinquency was silent");

    // Out of the due pool: later fires attempt nothing at all.
    let before = journal_len(&tw);
    sweep_at(&mut tw, 400);
    sweep_at(&mut tw, 500);
    assert_eq!(
        journal_len(&tw),
        before,
        "a delinquent obligation kept being attempted"
    );
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
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {}={} 1000", r.faucet, r.pat_acct),
    );
    ok(&mut tw, r.quinn, &format!("@obligation/reinstate {id}"));

    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active);
    assert!(
        o.schedule.next_due > 300,
        "the schedule was not re-anchored past now"
    );
    let closed = o.periods_settled;

    // Reinstating charges nothing for the time it spent delinquent.
    let before = bal(&tw, r.pat_acct);
    sweep_at(&mut tw, 350);
    assert_eq!(
        bal(&tw, r.pat_acct),
        before,
        "reinstating charged the delinquent gap"
    );
    sweep_at(&mut tw, 450);
    assert_eq!(
        bal(&tw, r.pat_acct),
        before - 100,
        "the next period did not settle"
    );
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
    assert_eq!(
        bal(&tw, r.pat_acct),
        10_000 - 300,
        "a downtime backlog was charged"
    );
    assert_eq!(
        o.status,
        ObligationStatus::Active,
        "forgiveness is not a wait state"
    );
    assert_eq!(
        o.periods_settled, 10,
        "the period index did not advance past the forgiven span"
    );

    let forgiven: Vec<&ObligationEvent> = o
        .history
        .iter()
        .filter(|e| matches!(e, ObligationEvent::Forgiven { .. }))
        .collect();
    assert_eq!(
        forgiven.len(),
        1,
        "the excess was forgiven in pieces: {forgiven:?}"
    );
    assert!(matches!(
        forgiven[0],
        ObligationEvent::Forgiven {
            periods: 7,
            cause: ForgiveCause::Downtime,
            ..
        }
    ));
    let said = texts(&ev);
    assert_eq!(
        said.matches("were forgiven").count(),
        2,
        "not exactly one notice per party: {said}"
    );
    assert!(
        said.contains("downtime"),
        "the notice did not carry the cause: {said}"
    );

    // Back on schedule, one period at a time.
    let before = bal(&tw, r.pat_acct);
    sweep_at(&mut tw, 1_100);
    assert_eq!(
        bal(&tw, r.pat_acct),
        before - 100,
        "the schedule did not resume"
    );
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
    tw.world_mut().set_game_clock(mush_core::GameClock {
        ic_ratio: 1,
        epoch_offset: 100_000,
    });
    sweep_at(&mut tw, 130);

    let o = obligation(&tw, id);
    assert!(o.forgiven_total > 0, "the jump was not forgiven");
    assert!(
        o.history.iter().any(|e| matches!(
            e,
            ObligationEvent::Forgiven {
                cause: ForgiveCause::TimeJump,
                ..
            }
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

    assert_eq!(
        bal(&tw, r.pat_acct),
        before,
        "a rewound clock charged something"
    );
    assert_eq!(
        journal_len(&tw),
        entries,
        "a rewound clock journaled something"
    );
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
            ObligationEvent::Forgiven {
                cause: ForgiveCause::Overload,
                ..
            }
        )),
        "the first starvation was misclassified: {:?}",
        o.history
    );
    assert_eq!(o.status, ObligationStatus::Active, "one overload escalated");
    assert_eq!(
        store(&tw).escalations().count(),
        0,
        "one overload paged a human"
    );

    sweep_at(&mut tw, 160);
    let o = obligation(&tw, id);
    assert!(
        matches!(
            o.status,
            ObligationStatus::Suspended {
                reason: SuspendReason::Escalated(_)
            }
        ),
        "a repeated overload did not suspend the subject: {:?}",
        o.status
    );
    let alert = store(&tw)
        .escalations()
        .next()
        .expect("an escalation")
        .clone();
    assert_eq!(alert.kind, EscalationKind::RepeatedOverload);
    assert_eq!(alert.subject, EscalationSubject::Obligation(id));
    assert!(
        !alert.recommended.is_empty(),
        "the alert carries no recommended command"
    );

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
    assert!(
        !store(&tw).is_frozen(),
        "a lagging container froze the economy"
    );
    assert_eq!(bal(&tw, r.pat_acct), 900, "the journal lost the settlement");
    assert_eq!(
        obligation(&tw, id).periods_settled,
        0,
        "the doc was not the stale one"
    );

    // The next fire re-derives the SAME deterministic key, resolves to
    // `AlreadyPosted`, and closes the period without charging it twice.
    sweep_at(&mut tw, 130);
    assert_eq!(
        bal(&tw, r.pat_acct),
        900,
        "the restart charged the period twice"
    );
    assert_eq!(
        obligation(&tw, id).periods_settled,
        1,
        "the period never closed"
    );
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
    assert_eq!(
        bal(&tw, r.pat_acct),
        1_000,
        "the sweep advanced a manual clock"
    );

    // The debtor cannot charge themselves less by refusing to step it, and cannot
    // step it either: advancing a manual clock is the act of charging.
    err(&mut tw, r.pat, &format!("@obligation/advance {id}"));
    ok(&mut tw, r.quinn, &format!("@obligation/advance {id}"));
    sweep_at(&mut tw, 1_100);
    assert_eq!(
        bal(&tw, r.pat_acct),
        900,
        "the stepped period did not settle"
    );
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
    assert!(
        store(tw).is_frozen(),
        "the durable freeze latch did not survive the reboot"
    );
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
    assert_eq!(
        obligation(&tw, id).periods_settled,
        0,
        "the sweep settled while frozen"
    );
    assert_eq!(
        store(&tw).last_sweep(),
        (0, 0),
        "a sweep that did nothing stamped anyway"
    );

    let alerts: Vec<_> = store(&tw).escalations().cloned().collect();
    assert_eq!(alerts.len(), 1, "expected exactly one record: {alerts:?}");
    assert_eq!(alerts[0].kind, EscalationKind::EconomyFrozen);
    assert_eq!(alerts[0].subject, EscalationSubject::Economy);
    // §4.6: the freeze itself is the level-3 action; the record it opens is an
    // ordinary level-2 page carrying the diagnosis and the recovery command.
    assert_eq!(alerts[0].level, 2);
    assert!(
        alerts[0]
            .recommended
            .iter()
            .any(|c| c.contains("@econ/unfreeze")),
        "the record does not carry the recovery command: {:?}",
        alerts[0].recommended
    );

    // Later fires do not mint a second record for the same open subject.
    sweep_at(&mut tw, 200);
    sweep_at(&mut tw, 300);
    assert_eq!(
        store(&tw).escalations().count(),
        1,
        "the freeze minted an alert storm"
    );

    let seen = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        seen.contains("economy frozen"),
        "the alert did not list: {seen}"
    );
    // Acknowledging the diagnosis is not the same act as clearing the freeze.
    let seen = ok(&mut tw, GOD, "@econ/ack 0");
    assert!(
        seen.contains("does not clear"),
        "ack implied it unfroze: {seen}"
    );
    assert!(
        store(&tw).is_frozen(),
        "@econ/ack cleared the whole-economy freeze"
    );
}

#[test]
fn the_escalation_allocator_floor_survives_a_lost_leaf_doc() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let _ = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);
    freeze(&mut tw, &mut kv);
    sweep_at(&mut tw, 100);
    assert!(
        store(&tw)
            .escalation(mush_economy::EscalationId(0))
            .is_some()
    );

    // The leaf doc is lost — but its id was minted, and an id that was minted must
    // never be minted again onto a different alert, or an acknowledgement would
    // resume the wrong subject.
    merge(&mut kv, drain(&mut tw));
    drop_key(&mut kv, "escalation:0");
    reboot(&mut tw, kv);
    assert!(
        store(&tw)
            .escalation(mush_economy::EscalationId(0))
            .is_none()
    );

    sweep_at(&mut tw, 200);
    let ids: Vec<u64> = store(&tw).escalations().map(|e| e.id.0).collect();
    assert_eq!(
        ids,
        vec![1],
        "a collected escalation id was re-minted: {ids:?}"
    );
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
        &format!(
            "@obligation/create {}={} 50 every=100 start=100",
            r.pat_acct, landlord_acct
        ),
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
        ObligationStatus::Suspended {
            reason: SuspendReason::Escalated(id),
        } => *id,
        other => panic!("the broken obligation was not suspended: {other:?}"),
    };
    assert!(
        !store(&tw).is_frozen(),
        "one broken creditor stopped the world"
    );
    assert_eq!(
        obligation(&tw, healthy).settled_total,
        1,
        "the healthy obligation was stopped too"
    );
    assert!(
        texts(&ev).contains("suspended"),
        "the parties were not told: {}",
        texts(&ev)
    );

    let alert = store(&tw).escalation(alert_id).expect("the alert").clone();
    assert_eq!(alert.kind, EscalationKind::SettlementRefused);
    assert_eq!(alert.subject, EscalationSubject::Obligation(broken));
    assert!(
        alert
            .recommended
            .iter()
            .any(|c| c.contains("@obligation/repoint") && c.contains("credit")),
        "the alert does not name the fix: {:?}",
        alert.recommended
    );
    let listed = ok(&mut tw, GOD, &format!("@econ/alerts {alert_id}"));
    assert!(
        listed.contains("@obligation/repoint"),
        "the alert did not print its fix: {listed}"
    );

    // The sweep does not keep attempting it (it is out of the due pool)...
    let entries = journal_len(&tw);
    sweep_at(&mut tw, 200);
    assert_eq!(
        journal_len(&tw),
        entries + 1,
        "only the healthy obligation should have posted"
    );
    assert_eq!(
        store(&tw).escalations().count(),
        1,
        "the suspended obligation kept paging"
    );

    // ...and the recommended command plus an acknowledgement puts it back.
    ok(
        &mut tw,
        GOD,
        &format!("@obligation/repoint {broken}=credit {}", r.quinn_acct),
    );
    ok(&mut tw, GOD, &format!("@econ/ack {alert_id}"));
    assert_eq!(obligation(&tw, broken).status, ObligationStatus::Active);
    let before = bal(&tw, r.pat_acct);
    sweep_at(&mut tw, 300);
    assert!(
        bal(&tw, r.pat_acct) < before,
        "the resumed obligation did not settle"
    );
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
        &format!(
            "@obligation/create {}={} 50 every=100 start=100",
            r.pat_acct, landlord_acct
        ),
    );
    let id = only_obligation(&tw);
    ok(&mut tw, GOD, &format!("@destroy #{}", landlord.0));

    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);

    assert!(
        !store(&tw).is_frozen(),
        "boot froze the world over one broken record"
    );
    let o = obligation(&tw, id);
    assert!(
        matches!(
            o.status,
            ObligationStatus::Suspended {
                reason: SuspendReason::Escalated(_)
            }
        ),
        "boot did not suspend an unsettleable obligation: {:?}",
        o.status
    );
    // Never deleted: the record is the only thing that remembers the debt.
    assert!(snap(&tw).obligation(id).is_some());
    let alert = store(&tw)
        .escalations()
        .next()
        .expect("boot opened no alert")
        .clone();
    assert_eq!(alert.kind, EscalationKind::BootReconciliation);
    assert!(
        alert
            .recommended
            .iter()
            .any(|c| c.contains("@obligation/repoint"))
    );
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
                EconomicCause::Staff {
                    reason: "re-entry".into(),
                },
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
        Box::new(mush_economy::EconomyModule::with_custody(vec![
            stub.clone(),
        ])),
    ]);
    let r = rig(&mut tw, 1_000);
    *stub.0.lock().expect("lock") = Some((r.faucet, r.quinn_acct));
    let chest = thing(&mut tw, "chest");
    let widget = thing(&mut tw, "widget");
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create box=container:#{}", chest.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", widget.0, r.pat.0),
    );

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
        Box::new(mush_economy::EconomyModule::with_custody(vec![
            stub.clone(),
        ])),
    ]);
    let r = rig(&mut tw, 1_000);
    let chest = thing(&mut tw, "chest");
    let widget = thing(&mut tw, "widget");
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create box=container:#{}", chest.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", widget.0, r.pat.0),
    );
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
    assert_eq!(
        store(&tw).last_sweep(),
        (100, 100),
        "the sweep did not stamp itself"
    );
    // And the stamps are durable, so the classifier's evidence survives a restart.
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert_eq!(
        store(&tw).last_sweep(),
        (100, 100),
        "the sweep stamps were not persisted"
    );
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
    assert_eq!(
        journal_len(&tw),
        entries,
        "a suspended obligation kept being attempted"
    );

    ok(&mut tw, GOD, &format!("@obligation/resume {id}"));
    let ev = sweep_at(&mut tw, 500);
    // Three periods went by while it was suspended; with the default catch-up of 3
    // they are attempted, not charged as a lump — and anything past the cap is
    // forgiven, never charged (doc 08 §2).
    let o = obligation(&tw, id);
    assert!(
        o.settled_total <= 4,
        "resuming charged the whole suspended gap"
    );
    assert!(!has_error(&ev), "the resumed sweep errored: {}", texts(&ev));
}

// ======================================================================
// ---- E4b1 verify fold ----
//
// One test per confirmed finding of the end-of-build adversarial verify, across
// its three lenses — durability/boot (B*), the sweep and the ladder (S*), and
// authority/disclosure (A*) — plus the owner's M1 decision on faucet-funded
// obligations. Each names the shape it pins, because the point of a regression
// test is to say what would otherwise come back.
//
// Two probes live here because the seams they exercise have no command surface:
// `@vf/post` posts a staged `EconomyPlan` (an optimistic-concurrency guard, a
// squatted idempotency key), and `@vf/mint` calls `create_system_obligation`,
// which is deliberately a crate API and not a verb.
// ======================================================================

#[derive(Default)]
struct VfState {
    plan: Option<EconomyPlan>,
    outcome: Option<Result<PostOutcome, EconomyError>>,
    spec: Option<(String, ObligationSpec)>,
    minted: Option<Result<ObligationId, EconomyError>>,
    /// Look-back fold: the last `register_instrument` outcome (`@vf/instrument`).
    registered: Option<Result<(), EconomyError>>,
    /// Look-back fold: the last `suspend_obligation` / `resume_obligation` outcome
    /// (`@vf/hold`, `@vf/release`), the release's `bool` folded in.
    seam: Option<Result<bool, EconomyError>>,
    /// Look-back fold, rent-after-end: the last `offer_lease` outcome (`@vf/lease`).
    lease: Option<Result<ContractId, ContractError>>,
    /// Look-back fold, rent-after-end: the last `end_lease` outcome (`@vf/endlease`).
    ended: Option<Result<LeaseEnd, ContractError>>,
    /// E6a step 1 (S3): the spec `@vf/create` hands to `create_obligation`.
    ruleset_spec: Option<ObligationSpec>,
    /// E6a step 1 (S3): the last `create_obligation` outcome.
    created: Option<Result<ObligationId, EconomyError>>,
    /// E6a step 1 (S3): the last `cancel_obligation` / `reinstate_obligation`
    /// outcome (`@vf/cancelob`, `@vf/restore`), the reinstate's `bool` folded in.
    ruleset: Option<Result<bool, EconomyError>>,
}

#[derive(Clone, Default)]
struct Vf(Arc<Mutex<VfState>>);

impl Vf {
    fn stage_post(&self, p: EconomyPlan) {
        self.0.lock().expect("probe lock").plan = Some(p);
    }
    fn outcome(&self) -> Result<PostOutcome, EconomyError> {
        self.0
            .lock()
            .expect("probe lock")
            .outcome
            .clone()
            .expect("an outcome was recorded")
    }
    fn stage_mint(&self, faucet: &str, spec: ObligationSpec) {
        self.0.lock().expect("probe lock").spec = Some((faucet.to_string(), spec));
    }
    fn minted(&self) -> Result<ObligationId, EconomyError> {
        self.0
            .lock()
            .expect("probe lock")
            .minted
            .clone()
            .expect("a mint outcome was recorded")
    }
    fn registered(&self) -> Result<(), EconomyError> {
        self.0
            .lock()
            .expect("probe lock")
            .registered
            .clone()
            .expect("a registration was recorded")
    }
    fn seam(&self) -> Result<bool, EconomyError> {
        self.0
            .lock()
            .expect("probe lock")
            .seam
            .clone()
            .expect("a seam outcome was recorded")
    }
    fn lease(&self) -> Result<ContractId, ContractError> {
        self.0
            .lock()
            .expect("probe lock")
            .lease
            .clone()
            .expect("a lease outcome was recorded")
    }
    fn ended(&self) -> Result<LeaseEnd, ContractError> {
        self.0
            .lock()
            .expect("probe lock")
            .ended
            .clone()
            .expect("an end_lease outcome was recorded")
    }
    fn stage_create(&self, spec: ObligationSpec) {
        self.0.lock().expect("probe lock").ruleset_spec = Some(spec);
    }
    fn created(&self) -> Result<ObligationId, EconomyError> {
        self.0
            .lock()
            .expect("probe lock")
            .created
            .clone()
            .expect("a create_obligation outcome was recorded")
    }
    fn ruleset(&self) -> Result<bool, EconomyError> {
        self.0
            .lock()
            .expect("probe lock")
            .ruleset
            .clone()
            .expect("a ruleset seam outcome was recorded")
    }
}

/// `#12`, `12` → 12.
fn probe_id(s: &str) -> u64 {
    s.trim()
        .trim_start_matches('#')
        .parse()
        .expect("a numeric id")
}

struct VfMod(Vf);

impl GameModule for VfMod {
    fn name(&self) -> &'static str {
        "vfprobe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        let st = self.0.clone();
        reg.command(
            CommandSpec {
                name: "@vf".into(),
                shape: ArgShape::EqPair,
                switches: [
                    "post",
                    "mint",
                    "instrument",
                    "hold",
                    "release",
                    "lease",
                    "endlease",
                    "create",
                    "cancelob",
                    "restore",
                ]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
                help: "verify-fold probe: post a staged plan / mint a staged system obligation / \
                       register an instrument / hold or release an obligation under a key / \
                       offer or end a lease through the layered-module seam / create, cancel or \
                       reinstate a ruleset obligation through the E6a seams"
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                // E6a step 1 (S3): `@vf/create` is `create_obligation` with the staged
                // spec under the ACTOR's authority; `@vf/cancelob <id>=<namespace>` and
                // `@vf/restore <id>=<namespace>` are the namespace-locked seams.
                if ctx.switches.has("create") {
                    let spec =
                        st.0.lock()
                            .expect("probe lock")
                            .ruleset_spec
                            .take()
                            .expect("stage a spec first");
                    let outcome = mush_economy::create_obligation(ctx, spec);
                    st.0.lock().expect("probe lock").created = Some(outcome);
                    return Ok(());
                }
                if ctx.switches.has("cancelob") || ctx.switches.has("restore") {
                    let id = ObligationId(probe_id(ctx.args.lhs()));
                    let namespace = ctx.args.rhs().unwrap_or_default().trim().to_string();
                    let outcome = if ctx.switches.has("cancelob") {
                        mush_economy::cancel_obligation(ctx, id, &namespace).map(|()| false)
                    } else {
                        mush_economy::reinstate_obligation(ctx, id, &namespace)
                    };
                    st.0.lock().expect("probe lock").ruleset = Some(outcome);
                    return Ok(());
                }
                // Look-back fold, rent-after-end: `@vf/lease <name>=<lessor> <tenant>
                // <premises> <from acct> <to acct>` is `offer_lease` (rent 100 every
                // 100 s from now), and `@vf/endlease <contract>=<lessor> <premises>`
                // is `end_lease` under that key.
                if ctx.switches.has("lease") {
                    let name = ctx.args.lhs().trim().to_string();
                    let rhs = ctx.args.rhs().unwrap_or_default().to_string();
                    let ids: Vec<u64> = rhs.split_whitespace().map(probe_id).collect();
                    let [lessor, tenant, premises, from, to] = ids[..] else {
                        return Err(CmdError::msg(
                            "@vf/lease <name>=<lessor> <tenant> <premises> <from> <to>",
                        ));
                    };
                    let lessor = mush_org::PrincipalId::Object(ObjId(
                        u32::try_from(lessor).expect("an obj id"),
                    ));
                    let tenant = mush_org::PrincipalId::Object(ObjId(
                        u32::try_from(tenant).expect("an obj id"),
                    ));
                    let now = ctx.w.now();
                    let spec = mush_economy::LeaseSpec {
                        name: SmolStr::new(name),
                        lessor: lessor.clone(),
                        scope: None,
                        tenants: vec![tenant.clone()],
                        premises: ObjId(u32::try_from(premises).expect("an obj id")),
                        rent: mush_economy::ObligationTemplate {
                            debtor: tenant,
                            creditor: lessor,
                            debit_account: AccountId(from),
                            credit_account: AccountId(to),
                            amount: Amount::from_minor(100),
                            schedule: EconomicSchedule::every(now.saturating_add(100), 100),
                            grace: GracePolicy::new(mush_economy::DEFAULT_GRACE_PERIODS),
                            notify_on_success: false,
                        },
                    };
                    let outcome = mush_economy::offer_lease(ctx, spec);
                    st.0.lock().expect("probe lock").lease = Some(outcome);
                    return Ok(());
                }
                if ctx.switches.has("endlease") {
                    let contract = ContractId(probe_id(ctx.args.lhs()));
                    let rhs = ctx.args.rhs().unwrap_or_default().to_string();
                    let ids: Vec<u64> = rhs.split_whitespace().map(probe_id).collect();
                    let [lessor, premises] = ids[..] else {
                        return Err(CmdError::msg("@vf/endlease <contract>=<lessor> <premises>"));
                    };
                    let key = mush_economy::LeaseKey {
                        lessor: mush_org::PrincipalId::Object(ObjId(
                            u32::try_from(lessor).expect("an obj id"),
                        )),
                        premises: ObjId(u32::try_from(premises).expect("an obj id")),
                    };
                    let outcome = mush_economy::end_lease(ctx, contract, &key);
                    st.0.lock().expect("probe lock").ended = Some(outcome);
                    return Ok(());
                }
                // Look-back fold, E2-C18: `@vf/instrument <code>` calls the API directly,
                // past the command's own guard.
                if ctx.switches.has("instrument") {
                    let code = ctx.args.lhs().trim().to_string();
                    let id = mush_economy::InstrumentId::new(&code).expect("a valid code");
                    let outcome = mush_economy::register_instrument(
                        ctx,
                        mush_economy::InstrumentDef {
                            id,
                            ruleset: SmolStr::new_static("probe"),
                            code: SmolStr::new(code),
                            kind: mush_economy::InstrumentKind::Currency,
                            scale: 0,
                            transferable: true,
                            negative_policy: mush_economy::NegativePolicy::SystemOnly,
                            status: mush_economy::InstrumentStatus::Active,
                            display: None,
                            revision: 1,
                        },
                    );
                    st.0.lock().expect("probe lock").registered = Some(outcome);
                    return Ok(());
                }
                // Look-back fold, D-LBK-1: `@vf/hold <obligation>=<key>` and
                // `@vf/release <obligation>=<key>` drive the layered-module seam.
                if ctx.switches.has("hold") || ctx.switches.has("release") {
                    let id = ObligationId(ctx.args.lhs().trim().parse().expect("an obligation id"));
                    let key = SuspensionKey::new(ctx.args.rhs().unwrap_or_default())
                        .expect("a valid key");
                    let outcome = if ctx.switches.has("hold") {
                        mush_economy::suspend_obligation(ctx, id, key).map(|()| false)
                    } else {
                        mush_economy::resume_obligation(ctx, id, &key)
                    };
                    st.0.lock().expect("probe lock").seam = Some(outcome);
                    return Ok(());
                }
                if ctx.switches.has("post") {
                    let plan =
                        st.0.lock()
                            .expect("probe lock")
                            .plan
                            .take()
                            .expect("stage a plan first");
                    let outcome = mush_economy::post(ctx, &plan);
                    st.0.lock().expect("probe lock").outcome = Some(outcome);
                    return Ok(());
                }
                let (faucet, spec) =
                    st.0.lock()
                        .expect("probe lock")
                        .spec
                        .take()
                        .expect("stage a spec first");
                let id = mush_org::SystemPrincipalId::new(&faucet).expect("valid faucet id");
                let outcome = mush_economy::create_system_obligation(ctx, &id, spec);
                st.0.lock().expect("probe lock").minted = Some(outcome);
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn boot_probe() -> (TestWorld, Vf) {
    boot_probe_with(EconomyConfig::default())
}

fn boot_probe_with(config: EconomyConfig) -> (TestWorld, Vf) {
    let vf = Vf::default();
    let tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![]).with_config(config)),
        Box::new(VfMod(vf.clone())),
    ]);
    (tw, vf)
}

/// A two-line staff plan with an explicit idempotency key — the shape both probes
/// need and no command offers.
fn staff_plan(key: IdempotencyKey, from: AccountId, to: AccountId, minor: i64) -> EconomyPlan {
    EconomyPlan::new(
        key,
        mush_org::PrincipalId::Object(GOD),
        EconomicCause::Staff {
            reason: "verify fold".into(),
        },
        vec![
            RequestedLine {
                account: from,
                delta: Amount::from_minor(-minor),
                memo: SmolStr::default(),
            },
            RequestedLine {
                account: to,
                delta: Amount::from_minor(minor),
                memo: SmolStr::default(),
            },
        ],
    )
}

/// Both namespaces, folded — an org reboot and an economy reboot are one event in
/// the real server, and rebooting only one would wipe the other's state.
type Kv2 = HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_all(tw: &mut TestWorld, acc: &mut Kv2) {
    for op in tw.take_module_ops() {
        let rows = acc.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                rows.retain(|(k, _)| *k != key);
                rows.push((key, value));
            }
            ModuleWrite::Del { key } => rows.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}

fn reboot_all(tw: &mut TestWorld, kv: &Kv2) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, rows)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: rows.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

fn revision_of(tw: &TestWorld, id: AccountId) -> u64 {
    snap(tw).account(id).expect("account").revision
}

fn alerts(tw: &TestWorld) -> Vec<mush_economy::Escalation> {
    store(tw).escalations().cloned().collect()
}

/// The one alert about `subject`, or a panic naming what there was instead.
fn alert_for(tw: &TestWorld, subject: EscalationSubject) -> mush_economy::Escalation {
    let found: Vec<_> = alerts(tw)
        .into_iter()
        .filter(|e| e.subject == subject)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one alert about {subject:?}, got {found:?}"
    );
    found.into_iter().next().expect("checked")
}

/// A Thing with an account and no money — a creditor that can be destroyed.
fn landlord(tw: &mut TestWorld, name: &str) -> (ObjId, AccountId) {
    let obj = thing(tw, name);
    ok(tw, GOD, &format!("@account/open #{}=gold", obj.0));
    let acct = account_of(tw, obj);
    (obj, acct)
}

// ---- Lens 1: durability and boot -------------------------------------------

/// B1 — a single key at `u64::MAX` must not take the world down, and must not
/// wrap the allocator back to zero either.
#[test]
fn vf_b1_an_allocator_key_at_the_ceiling_boots_and_mints_nothing() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    let _ = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);
    // A hand-written (or corrupt) leaf at the very top of the id space, and no
    // `meta` to floor the allocator from instead. `next_after`'s `m + 1` panicked
    // here in a debug build and wrapped to 0 in a release one — and a wrapped
    // escalation allocator re-mints live alert ids, so an acknowledgement resumes
    // the wrong subject.
    kv.push((
        SmolStr::new("escalation:18446744073709551615"),
        serde_json::json!({
            "id": 18_446_744_073_709_551_615u64,
            "level": 2,
            "kind": "SettlementRefused",
            "subject": { "Obligation": 999 },
            "context": "a record from the end of the id space",
            "recommended": [],
            "opened_at": 0,
            "acked_by": null,
            "acked_at": null,
            "notified_at": 0,
            "notifications": 0,
            "revision": 1
        }),
    ));
    drop_key(&mut kv, "meta");
    reboot(&mut tw, kv.clone());

    assert!(!store(&tw).is_frozen(), "a ceiling key froze the economy");
    assert_eq!(bal(&tw, r.pat_acct), 100, "boot lost the journal");

    // The allocator is parked AT the ceiling: a page that wants a fresh id gets
    // none, loudly, and mints nothing — rather than handing out a live id again.
    merge(&mut kv, drain(&mut tw));
    freeze(&mut tw, &mut kv);
    sweep_at(&mut tw, 100);
    let ids: Vec<u64> = store(&tw).escalations().map(|e| e.id.0).collect();
    assert_eq!(
        ids,
        vec![u64::MAX],
        "the ceiling allocator minted a colliding id: {ids:?}"
    );
}

/// B2 — the tick is infallible from dispatch's point of view, so an exhausted
/// allocator inside it must refuse the operation, not abort the world task.
#[test]
fn vf_b2_an_exhausted_alert_allocator_still_stops_the_subject() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let (obj, acct) = landlord(&mut tw, "escrow desk");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 50 every=100 start=100",
            r.pat_acct, acct
        ),
    );
    let id = only_obligation(&tw);

    // Park the escalation allocator at the ceiling BEFORE anything needs to page,
    // so boot itself reconciles nothing and the exhaustion is met in the tick.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, "meta", |v| {
        v["next_escalation_id"] = serde_json::json!(u64::MAX);
    });
    reboot(&mut tw, kv);

    // Now break the creditor, so the very next fire wants to page.
    ok(&mut tw, GOD, &format!("@destroy #{}", obj.0));
    let ev = sweep_at(&mut tw, 100);
    assert!(!has_error(&ev), "the sweep errored: {}", texts(&ev));

    // No record could be minted — and the subject stopped anyway, which is the half
    // that protects money.
    assert_eq!(
        store(&tw).escalations().count(),
        0,
        "an exhausted allocator minted a record"
    );
    let o = obligation(&tw, id);
    assert!(
        matches!(&o.status, ObligationStatus::Suspended { reason: SuspendReason::Staff { reason } }
            if reason.contains("allocator")),
        "the subject kept running with no alert to stop it: {:?}",
        o.status
    );
    assert!(
        !store(&tw).is_frozen(),
        "one exhausted allocator stopped the world"
    );
}

/// B3 — `revision` is journal authority, and a hold effect touches an account
/// exactly as a money line does.
#[test]
fn vf_b3_revision_does_not_rewind_for_hold_only_entries() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 300);
    // The account doc as it stood BEFORE any reservation, and the revision a guard
    // written at that moment would carry.
    let mut kv = fold_kv(&mut tw);
    let acct_key = format!("account:{}", r.pat_acct.0);
    let stale = value_of(&kv, &acct_key);
    let stale_revision = revision_of(&tw, r.pat_acct);

    // Three effect-only entries: two placements and a release. None of them moves a
    // single money line, and every one of them bumps `revision` in `apply`.
    ok(&mut tw, GOD, &format!("@hold/place {}=60 a", r.pat_acct));
    ok(&mut tw, GOD, &format!("@hold/place {}=90 b", r.pat_acct));
    ok(&mut tw, GOD, &format!("@hold/release {}", HoldId(1)));
    let live_revision = revision_of(&tw, r.pat_acct);
    assert_eq!(
        live_revision,
        stale_revision + 3,
        "apply did not bump once per hold entry"
    );

    // Crash: the holds are journaled, the account doc write dropped.
    merge(&mut kv, drain(&mut tw));
    patch(&mut kv, &acct_key, |v| *v = stale.clone());
    reboot(&mut tw, kv);

    assert!(
        !store(&tw).is_frozen(),
        "a lagging account doc froze the economy"
    );
    assert_eq!(
        revision_of(&tw, r.pat_acct),
        live_revision,
        "the reboot rewound `revision` past the hold entries"
    );

    // ...which is the whole point: a guard captured before the reservations must
    // still be Stale afterwards. Replaying only the money lines made it pass.
    let mut plan = staff_plan(
        IdempotencyKey::new(SmolStr::new_static("vf"), SmolStr::new_static("b3")),
        r.pat_acct,
        r.quinn_acct,
        1,
    );
    plan.expected_revisions = vec![AccountRevision {
        account: r.pat_acct,
        revision: stale_revision,
    }];
    vf.stage_post(plan);
    ok(&mut tw, GOD, "@vf/post");
    assert!(
        matches!(vf.outcome(), Err(EconomyError::Stale { .. })),
        "a stale guard passed after the restart: {:?}",
        vf.outcome()
    );
}

/// B4 (a) — recovery clears exactly what the latch froze, and the next sweep
/// settles normally with nobody paged.
#[test]
fn vf_b4_unfreeze_restores_only_the_accounts_the_latch_froze() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    // An account frozen for its OWN reason: its owner is gone, and no recovery
    // should hand it back.
    let (obj, sick) = landlord(&mut tw, "abandoned desk");
    ok(&mut tw, GOD, &format!("@destroy #{}", obj.0));
    assert_eq!(
        snap(&tw).account(sick).expect("account").status,
        AccountStatus::Frozen
    );

    let mut kv = fold_kv(&mut tw);
    freeze(&mut tw, &mut kv);
    assert_eq!(
        snap(&tw).account(r.pat_acct).expect("account").status,
        AccountStatus::Frozen
    );

    ok(&mut tw, GOD, "@econ/unfreeze");
    assert_eq!(
        snap(&tw).account(r.pat_acct).expect("account").status,
        AccountStatus::Active,
        "recovery left the accounts it froze frozen"
    );
    assert_eq!(
        snap(&tw).account(r.quinn_acct).expect("account").status,
        AccountStatus::Active
    );
    assert_eq!(
        snap(&tw).account(sick).expect("account").status,
        AccountStatus::Frozen,
        "recovery cleared an account frozen for its own reason"
    );

    // ...and the economy simply works again: no refusal, no page.
    let ev = sweep_at(&mut tw, 100);
    assert!(
        !has_error(&ev),
        "the recovered sweep errored: {}",
        texts(&ev)
    );
    assert_eq!(
        obligation(&tw, id).settled_total,
        1,
        "the recovered obligation did not settle"
    );
    assert_eq!(
        store(&tw).escalations().count(),
        0,
        "recovery paged a human"
    );
}

/// B4 (b + c) — one blocked wallet is one decision: one record, naming every
/// obligation it stopped, recommending the unfreeze rather than a repoint.
#[test]
fn vf_b4_two_obligations_behind_one_frozen_account_open_one_record() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let (_, desk) = landlord(&mut tw, "escrow desk");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100 start=100",
            r.pat_acct, r.quinn_acct
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={desk} 20 every=100 start=100",
            r.pat_acct
        ),
    );

    // Freeze the shared debtor. (Its owner is alive: this is a recoverable freeze,
    // not a destroyed principal, and the two have different right answers.)
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("account:{}", r.pat_acct.0), |v| {
        v["status"] = serde_json::json!("Frozen");
    });
    reboot(&mut tw, kv);

    sweep_at(&mut tw, 100);
    let records = alerts(&tw);
    assert_eq!(
        records.len(),
        1,
        "one frozen wallet paged twice: {records:?}"
    );
    let record = &records[0];
    assert_eq!(
        record.blamed,
        Some(r.pat_acct),
        "the record does not name the account it is about"
    );
    assert!(
        record.context.contains("obligation #1"),
        "the record does not name the other obligation it stopped: {}",
        record.context
    );
    assert!(
        record
            .recommended
            .iter()
            .any(|c| c.contains("@account/unfreeze")),
        "the advice sends staff to repoint a recoverable account: {:?}",
        record.recommended
    );
    assert!(
        !record
            .recommended
            .iter()
            .any(|c| c.contains("@obligation/repoint")),
        "the advice recommends abandoning the money in a frozen wallet: {:?}",
        record.recommended
    );
    // Both subjects are stopped, on the one record.
    for id in [ObligationId(0), ObligationId(1)] {
        assert!(
            matches!(obligation(&tw, id).status,
                ObligationStatus::Suspended { reason: SuspendReason::Escalated(e) } if e == record.id),
            "obligation {id} is not attached to the one open record"
        );
    }
}

/// B5 — `held_amount: 0` was the blind spot: a doc claiming a hold sequence it
/// never appears at read as a legitimately empty reservation.
#[test]
fn vf_b5_a_hold_anchor_the_account_never_appears_at_freezes() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    // A second journal entry, so sequence 1 is a REAL sequence to claim.
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {}={} 10", r.pat_acct, r.quinn_acct),
    );
    let mut kv = fold_kv(&mut tw);
    // Quinn's account has no hold effect anywhere in the journal, and says it was
    // last reconciled at sequence 1 with nothing reserved.
    patch(&mut kv, &format!("account:{}", r.quinn_acct.0), |v| {
        v["last_hold_seq"] = serde_json::json!(1);
        v["held_amount"] = serde_json::json!(0);
    });
    reboot(&mut tw, kv);

    assert!(
        store(&tw).is_frozen(),
        "a hold anchor the account never appears at did not freeze the economy"
    );
    assert_eq!(
        held(&tw, r.quinn_acct),
        0,
        "the impossible projection was not healed"
    );
}

/// B6 — a clock nobody can read never makes a payment due, which is silence, not
/// safety.
#[test]
fn vf_b6_an_obligation_on_a_missing_cadence_is_suspended_and_paged() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);
    // Re-point the schedule at a cadence clock that has no record — a campaign
    // clock deleted, or a doc written by a ruleset that never created it.
    patch(&mut kv, "obligation:0", |v| {
        v["schedule"]["clock"] = serde_json::json!({ "Cadence": 5 });
    });
    reboot(&mut tw, kv);

    let o = obligation(&tw, id);
    assert!(
        matches!(
            o.status,
            ObligationStatus::Suspended {
                reason: SuspendReason::Escalated(_)
            }
        ),
        "an obligation on an unreadable clock stayed Active forever: {:?}",
        o.status
    );
    let record = alert_for(&tw, EscalationSubject::Obligation(id));
    assert_eq!(record.kind, EscalationKind::BootReconciliation);
    assert!(
        record.context.contains("cadence clock #5"),
        "the diagnosis does not name the missing clock: {}",
        record.context
    );
    assert!(
        record
            .recommended
            .iter()
            .any(|c| c.contains("cadence clock #5")),
        "the advice recommends a repoint that would fix nothing: {:?}",
        record.recommended
    );
}

/// B7 — the journal tail and the freeze latch have exactly one mutator each, and
/// the world reads them through accessors rather than by reaching into fields.
#[test]
fn vf_b7_the_journal_and_the_freeze_latch_are_read_through_accessors() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    let seq = store(&tw).journal().next_seq(mush_economy::JOURNAL_LOG);
    assert!(seq > 0, "the mint did not reach the journal");
    assert!(!store(&tw).is_frozen());

    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {}={} 10", r.pat_acct, r.quinn_acct),
    );
    assert_eq!(
        store(&tw).journal().next_seq(mush_economy::JOURNAL_LOG),
        seq + 1,
        "the accessor does not track the one appender"
    );
    let mut kv = fold_kv(&mut tw);
    freeze(&mut tw, &mut kv);
    assert!(
        store(&tw).is_frozen(),
        "the durable latch is not visible through its accessor"
    );
}

/// B8 — a doc-authority status write never touches a journal-authority field.
#[test]
fn vf_b8_the_destruction_freeze_leaves_the_journal_revision_alone() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    let (obj, acct) = landlord(&mut tw, "escrow desk");
    // Give it a journal history and then empty it, so the destroy gate lets go.
    ok(&mut tw, GOD, &format!("@econ/pay {}={acct} 50", r.pat_acct));
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {acct}={} 50", r.quinn_acct),
    );
    let before = revision_of(&tw, acct);

    ok(&mut tw, GOD, &format!("@destroy #{}", obj.0));
    assert_eq!(
        snap(&tw).account(acct).expect("account").status,
        AccountStatus::Frozen
    );
    assert_eq!(
        revision_of(&tw, acct),
        before,
        "the destruction backstop bumped a journal-authority field"
    );

    // ...and the untouched revision is exactly what the journal re-derives, so the
    // freeze can never read as drift at the next boot.
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert!(
        !store(&tw).is_frozen(),
        "the doc-authority freeze read as drift at boot"
    );
    assert_eq!(
        snap(&tw).account(acct).expect("account").status,
        AccountStatus::Frozen
    );
    assert_eq!(revision_of(&tw, acct), before);
}

// ---- Lens 2: the sweep and the ladder --------------------------------------

/// S1 — the sweep is not exempt from the rules every manual payment obeys.
#[test]
fn vf_s1_a_suspended_org_stops_settlement_in_both_directions() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    ok(&mut tw, GOD, "@org/create Ironhold");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org store")
        .state()
        .orgs()
        .next()
        .expect("the org")
        .id;
    ok(&mut tw, GOD, "@account/open org:Ironhold=gold");
    let org_acct = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, mush_org::PrincipalId::Organization(_)))
        .expect("the org wallet")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {}={org_acct} 1000", r.faucet),
    );
    // The org owes Pat, and Pat owes the org: settlement in both directions.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {org_acct}={} 10 every=100 start=100",
            r.pat_acct
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={org_acct} 20 every=100 start=100",
            r.pat_acct
        ),
    );

    let mut kv: Kv2 = HashMap::new();
    fold_all(&mut tw, &mut kv);
    let key = mush_org::org_key(org);
    for (k, v) in kv.get_mut("org").expect("org rows") {
        if *k == key {
            v["status"] = serde_json::json!("Suspended");
        }
    }
    reboot_all(&mut tw, &kv);

    let (before_org, before_pat) = (bal(&tw, org_acct), bal(&tw, r.pat_acct));
    sweep_at(&mut tw, 100);
    assert_eq!(
        bal(&tw, org_acct),
        before_org,
        "a suspended org kept settling"
    );
    assert_eq!(
        bal(&tw, r.pat_acct),
        before_pat,
        "a suspended org kept collecting"
    );
    // One blocked wallet, one decision.
    let records = alerts(&tw);
    assert_eq!(
        records.len(),
        1,
        "the two directions paged separately: {records:?}"
    );
    assert_eq!(records[0].blamed, Some(org_acct));
}

/// S2 — the per-fire cap is a bound on work, not on iteration.
#[test]
fn vf_s2_the_cap_counts_only_cadences_with_work_to_do() {
    let mut tw = boot();
    let _ = rig(&mut tw, 100);
    let mut kv = fold_kv(&mut tw);
    // 64 clocks that cannot possibly have elapsed, and one that has. Taking the cap
    // over the whole list spent it on 64 no-ops and never reached the 65th.
    for i in 0..65u64 {
        let advanced_at = if i == 64 { 0 } else { 1_000_000 };
        kv.push((
            SmolStr::new(format!("cadence:{i}")),
            serde_json::json!({
                "id": i,
                "name": format!("clock{i}"),
                "source": "RealUtc",
                "period": 10,
                "catch_up_cap": 3,
                "position": 0,
                "advanced_at": advanced_at,
                "revision": 1
            }),
        ));
    }
    reboot(&mut tw, kv);
    assert_eq!(snap(&tw).cadences().count(), 65);

    sweep_at(&mut tw, 1_000);
    assert_eq!(
        snap(&tw)
            .cadence(CadenceId(64))
            .expect("the 65th clock")
            .position,
        3,
        "the only clock with work to do was starved by 64 no-ops"
    );
    // ...and the sleeping ones are untouched, so the filter did not simply advance
    // everything.
    assert_eq!(
        snap(&tw).cadence(CadenceId(0)).expect("clock 0").position,
        0
    );
}

/// S3 — a restart is downtime however short it was, and the no-boot threshold is
/// two sweep periods rather than four.
#[test]
fn vf_s3_a_restart_sized_gap_is_downtime_not_overload() {
    let mut tw = boot();
    let r = rig(&mut tw, 10_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=15 start=45 catchup=1",
            r.pat_acct, r.quinn_acct
        ),
    );
    let id = only_obligation(&tw);
    // The tick is armed one sweep period out, so 45 is the first instant a fire can
    // land on; the schedule is anchored there to match.
    sweep_at(&mut tw, 45);
    assert_eq!(
        obligation(&tw, id).settled_total,
        1,
        "the first period did not settle"
    );

    // A 45-second outage — SHORTER than the no-boot threshold — across a restart.
    // Without the boot marker this classified `Overload`, and `Overload` twice in
    // the window suspends a healthy obligation and pages a staffer.
    let kv = fold_kv(&mut tw);
    tw.set_now(90);
    reboot(&mut tw, kv);
    sweep_at(&mut tw, 90);
    let causes = forgive_causes(&tw, id);
    assert!(
        !causes.is_empty(),
        "the gap forgave nothing: {:?}",
        obligation(&tw, id).history
    );
    assert!(
        causes.iter().all(|c| *c == ForgiveCause::Downtime),
        "a restart was read as a starved post budget: {causes:?}"
    );

    // A second gap, 90 seconds and no restart: still downtime, on the threshold
    // alone. Two of these used to be two Overloads inside one window.
    sweep_at(&mut tw, 180);
    let o = obligation(&tw, id);
    let causes = forgive_causes(&tw, id);
    assert!(
        causes.iter().all(|c| *c == ForgiveCause::Downtime),
        "an ordinary outage was read as overload: {causes:?}"
    );
    assert_eq!(
        o.status,
        ObligationStatus::Active,
        "two outages suspended a healthy obligation"
    );
    assert_eq!(
        store(&tw).escalations().count(),
        0,
        "two outages paged a human"
    );
}

fn forgive_causes(tw: &TestWorld, id: ObligationId) -> Vec<ForgiveCause> {
    obligation(tw, id)
        .history
        .iter()
        .filter_map(|e| match e {
            ObligationEvent::Forgiven { cause, .. } => Some(*cause),
            _ => None,
        })
        .collect()
}

/// S4 — an acknowledgement is answered, not overwritten: one record per subject,
/// reopened when its cause comes back, never re-minted.
#[test]
fn vf_s4_an_acknowledged_alert_reopens_instead_of_re_minting() {
    // (a) The freeze record is one per freeze, however many times it is acked.
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let _ = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);
    freeze(&mut tw, &mut kv);
    sweep_at(&mut tw, 100);
    ok(&mut tw, GOD, "@econ/ack 0");
    sweep_at(&mut tw, 200);
    err(&mut tw, GOD, "@econ/ack 0");
    sweep_at(&mut tw, 300);
    err(&mut tw, GOD, "@econ/ack 0");
    assert_eq!(
        store(&tw).escalations().count(),
        1,
        "an acknowledged freeze minted a fresh alert on every fire"
    );

    // (b) An obligation acked without fixing its cause reopens the SAME record,
    // once, after the cool-off.
    let mut tw = boot();
    let r = rig(&mut tw, 10_000);
    let (obj, acct) = landlord(&mut tw, "escrow desk");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={acct} 50 every=100 start=100",
            r.pat_acct
        ),
    );
    let id = only_obligation(&tw);
    ok(&mut tw, GOD, &format!("@destroy #{}", obj.0));

    sweep_at(&mut tw, 100);
    let first = alert_for(&tw, EscalationSubject::Obligation(id)).id;
    // Acknowledged, but nothing was repointed: the cause is still there.
    ok(&mut tw, GOD, &format!("@econ/ack {first}"));
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Active);

    // Past the cool-off, the same record comes back — same id, with a tally.
    sweep_at(&mut tw, 100 + 3_600 + 60);
    let records = alerts(&tw);
    assert_eq!(
        records.len(),
        1,
        "the reopened cause minted a second alert: {records:?}"
    );
    assert_eq!(
        records[0].id, first,
        "the alert changed id under the staffer"
    );
    assert!(records[0].is_open(), "the record did not reopen");
    assert_eq!(
        records[0].reopened, 1,
        "the reopen tally is the only record of how often"
    );
}

/// S5 — `resume` now delivers the forgiveness its own notice promises.
#[test]
fn vf_s5_resume_forgives_the_suspended_gap_and_records_why() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.pat_acct), 900);

    ok(&mut tw, GOD, &format!("@obligation/suspend {id}=audit"));
    sweep_at(&mut tw, 200);
    sweep_at(&mut tw, 300);
    sweep_at(&mut tw, 400);
    // Look-back fold, D-LBK-1: the gap is forgiven **as it falls due**, by the sweep,
    // and recorded with its cause on each fire — so by the time anybody resumes there
    // is nothing left for the resumption to forgive, let alone charge.
    let held = obligation(&tw, id);
    assert_eq!(
        held.forgiven_total, 3,
        "the sweep did not forgive the suspended periods as they fell due"
    );
    assert!(
        held.history.iter().any(|e| matches!(
            e,
            ObligationEvent::Forgiven {
                cause: ForgiveCause::Suspension,
                ..
            }
        )),
        "the forgiveness was not recorded with its cause: {:?}",
        held.history
    );

    let seen = ok(&mut tw, GOD, &format!("@obligation/resume {id}"));
    assert!(
        seen.contains("No period fell due") || seen.contains("forgiven"),
        "resume said nothing about the gap: {seen}"
    );
    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active);
    assert!(
        o.schedule.next_due > 400,
        "the schedule was not re-anchored past the gap"
    );

    // ...and the next fire charges ONE period, not the whole suspension.
    let before = bal(&tw, r.pat_acct);
    sweep_at(&mut tw, 500);
    assert_eq!(
        bal(&tw, r.pat_acct),
        before - 100,
        "resuming charged the suspended gap"
    );
    assert_eq!(obligation(&tw, id).settled_total, 2);
}

/// S6 — reservations have a ladder now: a stuck expiry is somebody's money.
#[test]
fn vf_s6_a_hold_whose_expiry_keeps_failing_is_paged() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 1_000);
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 expires=10 a", r.pat_acct),
    );
    let hold = only_hold(&tw);

    // Squat the sweep's own deterministic expiry key with a different plan, so
    // every expiry attempt resolves to an idempotency CONFLICT and the reservation
    // can never age out. (E4b1 logged that at level 1 and retried forever.)
    vf.stage_post(staff_plan(
        IdempotencyKey::new(
            SmolStr::new_static("hold"),
            SmolStr::new(format!("{}/expire", hold.0)),
        ),
        r.faucet,
        r.quinn_acct,
        1,
    ));
    ok(&mut tw, GOD, "@vf/post");

    for t in [100, 200, 300] {
        sweep_at(&mut tw, t);
        assert_eq!(
            store(&tw).escalations().count(),
            0,
            "the ladder paged inside its own cap"
        );
    }
    sweep_at(&mut tw, 400);

    assert_eq!(snap(&tw).hold(hold).expect("hold").status, HoldStatus::Held);
    assert_eq!(
        held(&tw, r.pat_acct),
        60,
        "the money is still reserved, which is the point"
    );
    let record = alert_for(&tw, EscalationSubject::Hold(hold));
    assert!(
        record
            .recommended
            .iter()
            .any(|c| c.contains("@hold/release")),
        "the alert does not name a way out: {:?}",
        record.recommended
    );
    assert!(
        store(&tw).expiry_refusals(hold) >= 4,
        "the run was not counted"
    );
}

/// S8 — `grace = 255` was infinite grace, because `missed` saturates there.
#[test]
fn vf_s8_an_endless_grace_allowance_is_refused() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let seen = err(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100 grace=255",
            r.pat_acct, r.quinn_acct
        ),
    );
    assert!(
        seen.contains("254"),
        "the refusal does not say what the limit is: {seen}"
    );
    assert_eq!(
        snap(&tw).obligations().count(),
        0,
        "the refused create made a record"
    );

    // The largest allowance the state machine can actually end is accepted.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100 grace=254",
            r.pat_acct, r.quinn_acct
        ),
    );
    let id = only_obligation(&tw);
    assert_eq!(obligation(&tw, id).grace.periods, 254);
}

/// S9 — a documented limit, said out loud where it starts applying.
#[test]
fn vf_s9_an_approval_rule_on_the_debtor_is_flagged_at_creation() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    // A threshold a single period never reaches — the sub-threshold drip, which is
    // the half a staffer is least likely to notice on their own.
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=1000 threshold:1", r.pat_acct),
    );
    let seen = ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 100 every=100 start=100",
            r.pat_acct, r.quinn_acct
        ),
    );
    assert!(
        seen.contains("approval rule") && seen.contains("never escalates"),
        "the standing-order limit was not disclosed: {seen}"
    );
    // ...and the drip is real: period after period posts, none of them escalating.
    sweep_at(&mut tw, 100);
    sweep_at(&mut tw, 200);
    assert_eq!(
        bal(&tw, r.pat_acct),
        800,
        "the sweep did not settle under the threshold"
    );
    assert_eq!(
        snap(&tw).approvals().count(),
        0,
        "a sub-threshold period escalated"
    );
    assert_eq!(store(&tw).escalations().count(), 0);
}

// ---- Lens 3: authority, commands, existence hiding --------------------------

/// A1 — a reservation made TO someone is not the payer's to take back.
#[test]
fn vf_a1_debit_authority_does_not_release_a_third_partys_reservation() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 for=#{} escrow", r.pat_acct, r.quinn.0),
    );
    let hold = only_hold(&tw);

    // Pat owns the account and may spend every unreserved penny in it — and still
    // may not dissolve the promise standing on it. Escrow the payer can cancel is
    // not escrow.
    err(&mut tw, r.pat, &format!("@hold/release {hold}"));
    assert_eq!(
        held(&tw, r.pat_acct),
        60,
        "the payer released their own escrow"
    );
    assert_eq!(snap(&tw).hold(hold).expect("hold").status, HoldStatus::Held);

    // The beneficiary gives it back, which is the whole point of it being theirs.
    ok(&mut tw, r.quinn, &format!("@hold/release {hold}"));
    assert_eq!(held(&tw, r.pat_acct), 0);
}

/// A2 — seeing a reservation and being able to spend it are one predicate.
#[test]
fn vf_a2_whoever_acts_for_the_beneficiary_can_see_and_capture() {
    let mut tw = boot();
    let r = rig(&mut tw, 300);
    let nosy = tw.spawn_player("Nosy");
    ok(&mut tw, GOD, "@org/create Ironhold");
    ok(&mut tw, GOD, "@account/open org:Ironhold=gold");
    let org_acct = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, mush_org::PrincipalId::Organization(_)))
        .expect("the org wallet")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Ironhold economy.account.post", r.quinn.0),
    );

    // (a) An ORGANIZATION beneficiary. Nobody in it could see the escrow, so
    // nobody in it could capture it either.
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=60 for=org:Ironhold escrow", r.pat_acct),
    );
    let to_org = only_hold(&tw);
    let a = err(&mut tw, nosy, &format!("@hold/show {to_org}"));
    let b = err(&mut tw, nosy, "@hold/show 4242");
    assert_eq!(
        a, b,
        "a stranger can tell the org's escrow from a nonexistent hold"
    );
    ok(&mut tw, r.quinn, &format!("@hold/show {to_org}"));
    ok(
        &mut tw,
        r.quinn,
        &format!("@hold/capture {to_org}={org_acct} 60"),
    );
    assert_eq!(
        bal(&tw, org_acct),
        60,
        "the org's spender could not capture its own escrow"
    );

    // (b) A THING beneficiary — a shop counter, a guild strongbox — is acted for by
    // whoever controls it.
    let counter = tw
        .world_mut()
        .create(ObjKind::Thing, "shop counter", r.quinn, Some(LIMBO))
        .expect("create");
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=40 for=#{} layaway", r.pat_acct, counter.0),
    );
    let to_thing = HoldId(2);
    let c = err(&mut tw, nosy, &format!("@hold/show {to_thing}"));
    assert_eq!(
        c, b,
        "a stranger can tell the counter's escrow from a nonexistent hold"
    );
    ok(&mut tw, r.quinn, &format!("@hold/show {to_thing}"));
    ok(
        &mut tw,
        r.quinn,
        &format!("@hold/capture {to_thing}={} 40", r.quinn_acct),
    );
    assert_eq!(bal(&tw, r.quinn_acct), 40);
}

/// A3 — a `System` beneficiary is an internal settlement authority, never a
/// command's, and naming one is staff-only and registry-checked.
#[test]
fn vf_a3_a_system_beneficiary_is_staff_named_and_captured_by_nobody() {
    let mut tw = boot();
    let r = rig(&mut tw, 200);

    // A player cannot conjure one, and cannot use the refusal to enumerate which
    // system principals exist.
    let real = err(
        &mut tw,
        r.pat,
        &format!("@hold/place {}=10 for=sys:faucet x", r.pat_acct),
    );
    let fake = err(
        &mut tw,
        r.pat,
        &format!("@hold/place {}=10 for=sys:nothing x", r.pat_acct),
    );
    assert_eq!(
        real, fake,
        "the refusal distinguishes a registered faucet from a made-up one"
    );
    assert!(real.contains("No such principal."), "wrong refusal: {real}");
    assert_eq!(snap(&tw).holds().count(), 0);

    // Staff may name only a registered one...
    err(
        &mut tw,
        GOD,
        &format!("@hold/place {}=10 for=sys:nothing x", r.pat_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=10 for=sys:faucet escrow", r.pat_acct),
    );
    let hold = only_hold(&tw);

    // ...and then nobody, staff included, captures it from a command: the arm sits
    // ABOVE the staff short-circuit deliberately.
    err(
        &mut tw,
        GOD,
        &format!("@hold/capture {hold}={} 10", r.quinn_acct),
    );
    assert_eq!(
        bal(&tw, r.quinn_acct),
        0,
        "a staff command spent a system principal's escrow"
    );
    assert_eq!(held(&tw, r.pat_acct), 10);
    // The placer can still hand it back — nothing is stranded.
    ok(&mut tw, GOD, &format!("@hold/release {hold}"));
    assert_eq!(held(&tw, r.pat_acct), 0);
}

/// A4 — one decision per invocation on a manual clock.
#[test]
fn vf_a4_six_advances_charge_one_period() {
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

    ok(&mut tw, GOD, &format!("@obligation/advance {id}"));
    let seen = ok(&mut tw, GOD, &format!("@obligation/advance {id}"));
    assert!(
        seen.contains("Nothing moved"),
        "the second advance moved the clock again: {seen}"
    );
    for _ in 0..4 {
        ok(&mut tw, GOD, &format!("@obligation/advance {id}"));
    }

    sweep_at(&mut tw, 1_000);
    assert_eq!(
        bal(&tw, r.pat_acct),
        900,
        "six advances charged more than one period"
    );
    let o = obligation(&tw, id);
    assert_eq!(o.settled_total, 1);
    assert_eq!(
        o.forgiven_total, 0,
        "six advances built a backlog to forgive"
    );

    // The next period is a fresh decision, and it works.
    ok(&mut tw, GOD, &format!("@obligation/advance {id}"));
    sweep_at(&mut tw, 1_100);
    assert_eq!(bal(&tw, r.pat_acct), 800);
}

/// A5 — invariant 29 covers the whole obligation lifecycle, on both sides.
#[test]
fn vf_a5_close_and_reinstate_see_every_unfinished_obligation() {
    let mut tw = boot();
    let r = rig(&mut tw, 0);
    let (obj, acct) = landlord(&mut tw, "escrow desk");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={acct} 100 every=100 start=100",
            r.pat_acct
        ),
    );
    let id = only_obligation(&tw);

    // No funds: miss, grace, delinquency — out of the due pool, but not over.
    sweep_at(&mut tw, 100);
    sweep_at(&mut tw, 200);
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);

    // The debtor's account is empty and carries no hold, so only the obligation can
    // be what refuses the close.
    let seen = err(&mut tw, GOD, &format!("@account/close {}", r.pat_acct));
    assert!(
        seen.contains("Obligation"),
        "a delinquent obligation let its account close: {seen}"
    );
    assert_eq!(
        snap(&tw).account(r.pat_acct).expect("account").status,
        AccountStatus::Active
    );

    // And reinstating checks that there is still somewhere to settle.
    ok(&mut tw, GOD, &format!("@destroy #{}", obj.0));
    assert_eq!(
        snap(&tw).account(acct).expect("account").status,
        AccountStatus::Frozen
    );
    let seen = err(&mut tw, GOD, &format!("@obligation/reinstate {id}"));
    assert!(
        seen.contains("not active"),
        "reinstate ignored a dead creditor account: {seen}"
    );
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);

    // Repoint it and the same command works — the check is a redirection, not a wall.
    ok(
        &mut tw,
        GOD,
        &format!("@obligation/repoint {id}=credit {}", r.quinn_acct),
    );
    ok(&mut tw, GOD, &format!("@obligation/reinstate {id}"));
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Active);
}

/// A6 — an open alert is answered, not stepped around.
#[test]
fn vf_a6_resume_refuses_an_escalated_suspension() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let (obj, acct) = landlord(&mut tw, "escrow desk");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={acct} 50 every=100 start=100",
            r.pat_acct
        ),
    );
    let id = only_obligation(&tw);
    ok(&mut tw, GOD, &format!("@destroy #{}", obj.0));
    sweep_at(&mut tw, 100);
    let alert = alert_for(&tw, EscalationSubject::Obligation(id)).id;

    let seen = err(&mut tw, GOD, &format!("@obligation/resume {id}"));
    assert!(
        seen.contains(&format!("@econ/ack {alert}")),
        "resume did not point at the verb that answers the alert: {seen}"
    );
    assert!(
        matches!(
            obligation(&tw, id).status,
            ObligationStatus::Suspended { .. }
        ),
        "resume lifted an escalated suspension and left the alert open"
    );
    assert!(alert_for(&tw, EscalationSubject::Obligation(id)).is_open());

    // The one verb that does both still does both.
    ok(
        &mut tw,
        GOD,
        &format!("@obligation/repoint {id}=credit {}", r.quinn_acct),
    );
    ok(&mut tw, GOD, &format!("@econ/ack {alert}"));
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Active);
}

/// A7 — a deadline in the past is a typo, not a deadline.
#[test]
fn vf_a7_a_non_positive_expiry_is_refused() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    for spec in ["expires=0", "expires=-60", "expires=ic:0", "expires=ic:-5"] {
        let seen = err(
            &mut tw,
            GOD,
            &format!("@hold/place {}=10 {spec} a", r.pat_acct),
        );
        assert!(
            seen.contains("POSITIVE"),
            "wrong refusal for {spec}: {seen}"
        );
    }
    assert_eq!(
        snap(&tw).holds().count(),
        0,
        "a refused placement reserved money"
    );
    assert_eq!(held(&tw, r.pat_acct), 0);

    ok(
        &mut tw,
        GOD,
        &format!("@hold/place {}=10 expires=60 a", r.pat_acct),
    );
    assert_eq!(held(&tw, r.pat_acct), 10);
}

/// A8 — the help is the surface most staff ever read.
#[test]
fn vf_a8_the_obligation_help_documents_what_the_verbs_actually_do() {
    let mut tw = boot();
    let seen = ok(&mut tw, GOD, "help @obligation");
    assert!(
        seen.contains("manual:"),
        "the manual clock is undocumented: {seen}"
    );
    assert!(
        seen.contains("@econ/alerts"),
        "the alert surface is undocumented: {seen}"
    );
    assert!(
        seen.contains("creditor"),
        "who may advance/reinstate is undocumented: {seen}"
    );
    assert!(
        seen.contains("0-254"),
        "the grace bound is undocumented: {seen}"
    );
}

// ---- M1: faucet-funded obligations ------------------------------------------

/// M1 — a standing mint is a ruleset decision made in code, never a staff verb.
#[test]
fn vf_m1_a_faucet_obligation_exists_only_through_the_ruleset_entry() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 0);

    // The command refuses the shape, as it always did.
    let seen = err(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 100 every=100 start=100",
            r.faucet, r.quinn_acct
        ),
    );
    assert!(seen.contains("standing mint"), "wrong refusal: {seen}");
    assert_eq!(snap(&tw).obligations().count(), 0);

    // The ruleset's own entry creates it, stamped with the faucet that funds it.
    vf.stage_mint(
        "faucet",
        ObligationSpec {
            debit_account: r.faucet,
            credit_account: r.quinn_acct,
            amount: Amount::from_minor(100),
            schedule: EconomicSchedule::new(EconomicClock::RealUtc, 100, Some(100), 3),
            grace: GracePolicy::default(),
            notify_on_success: false,
            cause: ObligationCause::Ruleset {
                namespace: "wod.income".into(),
            },
        },
    );
    ok(&mut tw, GOD, "@vf/mint");
    let id = vf
        .minted()
        .expect("the ruleset entry refused a legitimate income line");
    assert_eq!(
        obligation(&tw, id).minted_by,
        Some(mush_org::PrincipalId::System(
            mush_org::SystemPrincipalId::new("faucet").expect("id")
        ))
    );

    // ...and it settles, which is the whole point: E5's income table needs this to
    // work, not merely to be refused safely.
    sweep_at(&mut tw, 100);
    assert_eq!(
        bal(&tw, r.quinn_acct),
        100,
        "the faucet obligation did not settle"
    );
    assert_eq!(obligation(&tw, id).settled_total, 1);

    // A faucet the world never registered mints nothing.
    vf.stage_mint(
        "ghost",
        ObligationSpec {
            debit_account: r.faucet,
            credit_account: r.quinn_acct,
            amount: Amount::from_minor(1),
            schedule: EconomicSchedule::every(1_000, 100),
            grace: GracePolicy::default(),
            notify_on_success: false,
            cause: ObligationCause::Ruleset {
                namespace: "wod.income".into(),
            },
        },
    );
    ok(&mut tw, GOD, "@vf/mint");
    assert_eq!(
        vf.minted(),
        Err(EconomyError::Unauthorized),
        "an unregistered faucet minted"
    );
}

/// M1 — the cap binds at creation AND at every settlement, because the record can
/// be edited after the fact and the config can be lowered afterwards.
#[test]
fn vf_m1_the_mint_cap_bounds_creation_and_every_settlement() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 0);
    let spec = |minor: i64| ObligationSpec {
        debit_account: r.faucet,
        credit_account: r.quinn_acct,
        amount: Amount::from_minor(minor),
        schedule: EconomicSchedule::new(EconomicClock::RealUtc, 100, Some(100), 3),
        grace: GracePolicy::default(),
        notify_on_success: false,
        cause: ObligationCause::Ruleset {
            namespace: "wod.income".into(),
        },
    };

    // `gold` has scale 0, so the default 50,000 major units is 50,000 minor.
    vf.stage_mint("faucet", spec(50_001));
    ok(&mut tw, GOD, "@vf/mint");
    assert_eq!(vf.minted(), Err(EconomyError::MintCapExceeded(r.faucet)));
    assert_eq!(
        snap(&tw).obligations().count(),
        0,
        "an over-cap mint left a record behind"
    );

    vf.stage_mint("faucet", spec(100));
    ok(&mut tw, GOD, "@vf/mint");
    let id = vf.minted().expect("an under-cap income line was refused");
    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.quinn_acct), 100);

    // Now edit the stored record above the cap — the shape a compromised ruleset,
    // a bad migration, or a hand-edited doc produces. Creation-time checking alone
    // would have paid it forever.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("obligation:{}", id.0), |v| {
        v["amount"] = serde_json::json!(60_000);
    });
    reboot(&mut tw, kv);
    let before = bal(&tw, r.quinn_acct);
    sweep_at(&mut tw, 200);

    assert_eq!(
        bal(&tw, r.quinn_acct),
        before,
        "a record edited above the cap kept minting"
    );
    let record = alert_for(&tw, EscalationSubject::Obligation(id));
    assert!(
        record.context.contains("mint cap"),
        "the alert does not say what the decision is: {}",
        record.context
    );
    assert!(
        matches!(
            obligation(&tw, id).status,
            ObligationStatus::Suspended { .. }
        ),
        "the over-cap record kept being attempted"
    );
}

/// M1 — a per-instrument override, for worlds running several currencies at
/// different denominations.
#[test]
fn vf_m1_a_per_instrument_cap_overrides_the_default() {
    let mut config = EconomyConfig::default();
    config.instruments.insert(
        "gold".to_string(),
        InstrumentCaps {
            mint_obligation_cap_major: 5,
        },
    );
    let (mut tw, vf) = boot_probe_with(config);
    let r = rig(&mut tw, 0);
    let spec = |minor: i64| ObligationSpec {
        debit_account: r.faucet,
        credit_account: r.quinn_acct,
        amount: Amount::from_minor(minor),
        schedule: EconomicSchedule::new(EconomicClock::RealUtc, 100, Some(100), 3),
        grace: GracePolicy::default(),
        notify_on_success: false,
        cause: ObligationCause::Ruleset {
            namespace: "wod.income".into(),
        },
    };

    // Well under the crate-wide 50,000, and over this instrument's 5.
    vf.stage_mint("faucet", spec(6));
    ok(&mut tw, GOD, "@vf/mint");
    assert_eq!(vf.minted(), Err(EconomyError::MintCapExceeded(r.faucet)));

    vf.stage_mint("faucet", spec(5));
    ok(&mut tw, GOD, "@vf/mint");
    let id = vf.minted().expect("the override refused its own ceiling");
    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.quinn_acct), 5);
    assert_eq!(obligation(&tw, id).settled_total, 1);
}

// ======================================================================
// ---- look-back fold: E2-C18 ----
// ======================================================================

/// `register_instrument` is create-only: a second registration of the same id is
/// refused at the API (not only by the command's guard) and the live definition is
/// untouched.
#[test]
fn lb_e2c18_register_instrument_refuses_an_existing_id() {
    let (mut tw, vf) = boot_probe();
    ok(&mut tw, GOD, "@instrument/register gold=2");
    let before = snap(&tw)
        .instrument(&mush_economy::InstrumentId::new("gold").unwrap())
        .cloned()
        .expect("gold");
    ok(&mut tw, GOD, "@vf/instrument gold");
    assert_eq!(
        vf.registered(),
        Err(EconomyError::InstrumentExists(
            mush_economy::InstrumentId::new("gold").unwrap()
        ))
    );
    let after = snap(&tw)
        .instrument(&mush_economy::InstrumentId::new("gold").unwrap())
        .cloned()
        .expect("gold");
    assert_eq!(
        after, before,
        "the refused registration replaced the live definition"
    );
    assert_eq!(after.scale, 2, "the scale was overwritten");
    // The command keeps its own readable refusal.
    let seen = err(&mut tw, GOD, "@instrument/register gold=0");
    assert!(seen.contains("already registered"), "{seen}");
    // A fresh id still registers through the API.
    ok(&mut tw, GOD, "@vf/instrument silver");
    assert_eq!(vf.registered(), Ok(()));
    assert!(
        snap(&tw)
            .instrument(&mush_economy::InstrumentId::new("silver").unwrap())
            .is_some()
    );
}

/// A `Suspended` instrument — reachable only through the document, since no
/// producer exists — refuses a post at the engine (`InactiveInstrument`), and nothing
/// moves.
#[test]
fn lb_e2c18_a_suspended_instrument_refuses_a_post_at_the_engine() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, "instrument:gold", |v| {
        v["status"] = serde_json::json!("Suspended");
    });
    reboot(&mut tw, kv);
    assert_eq!(
        snap(&tw)
            .instrument(&mush_economy::InstrumentId::new("gold").unwrap())
            .map(|i| i.status),
        Some(mush_economy::InstrumentStatus::Suspended)
    );
    let before = journal_len(&tw);
    let seen = err(
        &mut tw,
        r.pat,
        &format!("@econ/pay {}={} 10", r.pat_acct, r.quinn_acct),
    );
    assert!(seen.contains("is not active"), "wrong refusal: {seen}");
    assert_eq!(
        bal(&tw, r.pat_acct),
        100,
        "a suspended instrument still moved money"
    );
    assert_eq!(bal(&tw, r.quinn_acct), 0);
    assert_eq!(
        journal_len(&tw),
        before,
        "the refused post journaled something"
    );
}

// ======================================================================
// ---- look-back fold: D-LBK-7 ----
// ======================================================================

/// An old-shape cadence document (no `generation`, no `schema`) loads with
/// `generation = 1, schema = 1`, a round-trip preserves them, and
/// `@econ/cadence show` prints both.
#[test]
fn lb_dlbk7_an_old_cadence_document_loads_with_generation_1_and_schema_1() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@econ/cadence create chronicle source=manual");
    let mut kv = fold_kv(&mut tw);
    let doc = value_of(&kv, "cadence:0");
    assert_eq!(doc["generation"], serde_json::json!(1));
    assert_eq!(doc["schema"], serde_json::json!(1));
    // The pre-fold shape: neither field on the document.
    patch(&mut kv, "cadence:0", |v| {
        v.as_object_mut().expect("an object").remove("generation");
        v.as_object_mut().expect("an object").remove("schema");
    });
    reboot(&mut tw, kv);
    let c = snap(&tw)
        .cadence(CadenceId(0))
        .cloned()
        .expect("the cadence loaded");
    assert_eq!(c.generation, mush_economy::CADENCE_GENERATION_INITIAL);
    assert_eq!(c.schema, mush_economy::CADENCE_SCHEMA_LEGACY);
    assert_eq!(c.revision, 1, "the staleness stamp is not a generation");
    // A round-trip preserves them, and an advance moves `revision` alone.
    ok(&mut tw, GOD, "@econ/cadence advance 0 2");
    let kv = fold_kv(&mut tw);
    let doc = value_of(&kv, "cadence:0");
    assert_eq!(doc["generation"], serde_json::json!(1));
    assert_eq!(doc["schema"], serde_json::json!(1));
    // One explicit act, one revision bump — however many periods it covered.
    assert_eq!(doc["revision"], serde_json::json!(2));
    let back: mush_economy::CadenceClock = serde_json::from_value(doc).expect("round-trips");
    assert_eq!((back.generation, back.schema, back.position), (1, 1, 2));
    let seen = ok(&mut tw, GOD, "@econ/cadence show 0");
    assert!(seen.contains("generation 1 · schema 1"), "{seen}");
}

// ======================================================================
// ---- look-back fold: D-LBK-1 (the economy seam alone) ----
// ======================================================================

/// Two holders on one obligation: while either holds, no period is charged (each is
/// forgiven as it falls due, `Suspension`); releasing one keeps it suspended; releasing
/// both returns it to the due pool, and the next period charges one period.
#[test]
fn lb_dlbk1_the_seam_alone_two_keys_resume_one_keeps_it_suspended() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.pat_acct), 900);

    ok(
        &mut tw,
        GOD,
        &format!("@vf/hold {id}=housing.corrupt:room:5"),
    );
    assert_eq!(vf.seam(), Ok(false));
    ok(
        &mut tw,
        GOD,
        &format!("@vf/hold {id}=housing.corrupt:tenancy:7"),
    );
    assert_eq!(vf.seam(), Ok(false));
    let o = obligation(&tw, id);
    assert!(
        matches!(
            &o.status,
            ObligationStatus::Suspended { reason: SuspendReason::Held { key } } if key.as_str() == "housing.corrupt:room:5"
        ),
        "{:?}",
        o.status
    );
    assert_eq!(o.holders().len(), 2);
    assert!(o.history.iter().any(|e| matches!(e, ObligationEvent::Held { key, .. } if key.as_str() == "housing.corrupt:room:5")));
    // Idempotent: holding under a key already held changes nothing.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/hold {id}=housing.corrupt:room:5"),
    );
    assert_eq!(obligation(&tw, id).revision, o.revision);

    // Held: two periods fall due and are forgiven as they fall due, never charged.
    sweep_at(&mut tw, 200);
    sweep_at(&mut tw, 300);
    let held = obligation(&tw, id);
    assert_eq!(bal(&tw, r.pat_acct), 900, "a held obligation was charged");
    assert_eq!(held.forgiven_total, 2);
    assert!(held.history.iter().any(|e| matches!(
        e,
        ObligationEvent::Forgiven {
            cause: ForgiveCause::Suspension,
            ..
        }
    )));
    // The staff verb may not lift a module's hold, and `@obligation/show` names both.
    let seen = err(&mut tw, GOD, &format!("@obligation/resume {id}"));
    assert!(
        seen.contains("held out of the due pool by `housing.corrupt:room:5`"),
        "{seen}"
    );
    let seen = ok(&mut tw, GOD, &format!("@obligation/show {id}"));
    assert!(
        seen.contains("holders: housing.corrupt:room:5, housing.corrupt:tenancy:7"),
        "{seen}"
    );

    // Release one: still suspended, the rendered reason rotates to the other holder.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/release {id}=housing.corrupt:room:5"),
    );
    assert_eq!(
        vf.seam(),
        Ok(false),
        "releasing one of two holders returned it to the pool"
    );
    let o = obligation(&tw, id);
    assert!(
        matches!(
            &o.status,
            ObligationStatus::Suspended { reason: SuspendReason::Held { key } } if key.as_str() == "housing.corrupt:tenancy:7"
        ),
        "{:?}",
        o.status
    );
    assert_eq!(o.holders().len(), 1);
    sweep_at(&mut tw, 400);
    assert_eq!(bal(&tw, r.pat_acct), 900, "one holder left still charged");

    // Release both: back in the due pool; the next period charges exactly one.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/release {id}=housing.corrupt:tenancy:7"),
    );
    assert_eq!(vf.seam(), Ok(true));
    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active);
    assert!(o.holders().is_empty());
    assert!(
        o.history
            .iter()
            .any(|e| matches!(e, ObligationEvent::Resumed { .. }))
    );
    assert!(
        o.schedule.next_due > 400,
        "the schedule was not re-anchored past the gap"
    );
    sweep_at(&mut tw, 500);
    assert_eq!(
        bal(&tw, r.pat_acct),
        800,
        "the resumed obligation did not charge one period"
    );
    assert_eq!(obligation(&tw, id).settled_total, 2);
    // Releasing a key that holds nothing is a no-op.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/release {id}=housing.corrupt:tenancy:7"),
    );
    assert_eq!(vf.seam(), Ok(false));
}

/// The ladder's own page-suspension and a module's hold coexist: `@econ/ack`
/// releases the ladder's key alone and the obligation stays out of the pool until
/// the module releases; the module's release alone leaves the ladder's hold standing.
#[test]
fn lb_dlbk1_ack_and_a_module_hold_never_lift_each_other() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 1_000);
    let (obj, acct) = landlord(&mut tw, "escrow desk");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={acct} 50 every=100 start=100",
            r.pat_acct
        ),
    );
    let id = only_obligation(&tw);
    ok(&mut tw, GOD, &format!("@destroy #{}", obj.0));
    sweep_at(&mut tw, 100);
    let alert = alert_for(&tw, EscalationSubject::Obligation(id)).id;
    let o = obligation(&tw, id);
    assert!(
        matches!(o.status, ObligationStatus::Suspended { reason: SuspendReason::Escalated(a) } if a == alert)
    );
    assert!(o.held_by(&SuspensionKey::escalation(alert)));

    // A module takes its own hold on the paged obligation.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/hold {id}=housing.corrupt:orphan:3"),
    );
    assert_eq!(obligation(&tw, id).holders().len(), 2);
    // Acknowledging releases the ladder's key and says the obligation is still held.
    let seen = ok(&mut tw, GOD, &format!("@econ/ack {alert}"));
    assert!(
        seen.contains("still out of the due pool: it is held by housing.corrupt:orphan:3"),
        "{seen}"
    );
    let o = obligation(&tw, id);
    assert!(
        matches!(&o.status, ObligationStatus::Suspended { reason: SuspendReason::Held { key } } if key.as_str() == "housing.corrupt:orphan:3"),
        "{:?}",
        o.status
    );
    assert!(!o.held_by(&SuspensionKey::escalation(alert)));
    sweep_at(&mut tw, 200);
    assert_eq!(
        bal(&tw, r.pat_acct),
        1_000,
        "an acknowledged page charged over a module's hold"
    );

    // The module releases: back in the pool (the ladder's key was already gone).
    ok(
        &mut tw,
        GOD,
        &format!("@vf/release {id}=housing.corrupt:orphan:3"),
    );
    assert_eq!(vf.seam(), Ok(true));
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Active);

    // The other order cannot arise: an obligation a module holds is never attempted
    // by the sweep and is out of the pool at boot, so the ladder never pages it
    // while it is held — and the hold survives a reboot. Once released, the very
    // next sweep finds the real problem and pages it (the same record, reused).
    let (obj2, acct2) = landlord(&mut tw, "second desk");
    ok(
        &mut tw,
        GOD,
        &format!("@obligation/repoint {id}=credit {acct2}"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@vf/hold {id}=housing.corrupt:room:9"),
    );
    ok(&mut tw, GOD, &format!("@destroy #{}", obj2.0));
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    let o = obligation(&tw, id);
    assert!(
        o.held_by(&SuspensionKey::new("housing.corrupt:room:9").unwrap()),
        "a reboot lost a module's hold: {:?}",
        o.holders()
    );
    sweep_at(&mut tw, 300);
    assert_eq!(alerts(&tw).len(), 1, "a held obligation was paged");
    assert!(matches!(
        &obligation(&tw, id).status,
        ObligationStatus::Suspended {
            reason: SuspendReason::Held { .. }
        }
    ));
    ok(
        &mut tw,
        GOD,
        &format!("@vf/release {id}=housing.corrupt:room:9"),
    );
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Active);
    sweep_at(&mut tw, 400);
    let o = obligation(&tw, id);
    assert!(
        matches!(o.status, ObligationStatus::Suspended { reason: SuspendReason::Escalated(a) } if a == alert),
        "the released obligation was not paged about its destroyed creditor: {:?}",
        o.status
    );
    assert_eq!(bal(&tw, r.pat_acct), 1_000);
}

// ======================================================================
// ---- look-back fold: rent-after-end (the economy seam alone) ----
// ======================================================================
//
// `end_lease` is `offer_lease`'s counterpart: the layered module that offered a lease
// ends it, and the economy does the unwind under its own terminal-transition rule.
// Driven through the probe module here, with no housing in the world at all.

/// `end_lease` under the owner's key posts the unwind under `contract/{id}/refund/0`
/// exactly once and lands `Cancelled` with the rent `Cancelled` and the `Lease` claim
/// off the premises; a second call is `AlreadyEnded` with nothing posted; a replay
/// after a lost state write converges with no second post; and a key the contract
/// does not answer to — the wrong lessor, the wrong premises, a contract that is not a
/// lease, a contract that does not exist — is refused with nothing written.
#[test]
fn lb_rent_after_end_end_lease_posts_under_the_key_once_cancels_the_rent_and_refuses_a_foreign_key()
{
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 1_000);
    let flat = thing(&mut tw, "flat");
    let shed = thing(&mut tw, "shed");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", flat.0, r.quinn.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vf/lease Lease=#{} #{} #{} {} {}",
            r.quinn.0, r.pat.0, flat.0, r.pat_acct, r.quinn_acct
        ),
    );
    let id = vf.lease().expect("the lease was offered");
    let state = |tw: &TestWorld| snap(tw).contract(id).expect("the lease").state;
    let pending = |tw: &TestWorld| snap(tw).contract(id).expect("the lease").pending.clone();
    let claims = |tw: &TestWorld| snap(tw).asset(flat).expect("the flat").encumbrances.clone();
    let refunds = |tw: &TestWorld| {
        let key = mush_economy::refund_key(id);
        store(tw)
            .transactions()
            .filter(|t| t.idempotency == key)
            .count()
    };
    ok(&mut tw, r.pat, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw), ContractState::Active);
    assert_eq!(
        claims(&tw),
        vec![Encumbrance::Lease {
            lessor: mush_org::PrincipalId::Object(r.quinn),
            contract: id
        }]
    );
    let rent = snap(&tw)
        .obligations()
        .find(|o| o.cause == ObligationCause::Contract(id))
        .expect("the rent")
        .id;
    assert_eq!(obligation(&tw, rent).status, ObligationStatus::Active);
    sweep_at(&mut tw, 100);
    assert_eq!(
        bal(&tw, r.pat_acct),
        900,
        "one period of rent while the lease is live"
    );
    let posts = journal_len(&tw);

    // Foreign keys: refused, and nothing written — no state, no pending, no post.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease {id}=#{} #{}", r.pat.0, flat.0),
    );
    assert!(
        matches!(vf.ended(), Err(ContractError::Infeasible(_))),
        "the wrong lessor was allowed: {:?}",
        vf.ended()
    );
    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease {id}=#{} #{}", r.quinn.0, shed.0),
    );
    assert!(
        matches!(vf.ended(), Err(ContractError::Infeasible(_))),
        "the wrong premises were allowed: {:?}",
        vf.ended()
    );
    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease 999=#{} #{}", r.quinn.0, flat.0),
    );
    assert!(
        matches!(vf.ended(), Err(ContractError::Infeasible(_))),
        "a missing contract was allowed: {:?}",
        vf.ended()
    );
    ok(&mut tw, r.quinn, "@contract/draft Plain");
    let plain = snap(&tw)
        .contracts()
        .find(|c| c.name == "Plain")
        .expect("the draft")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease {plain}=#{} #{}", r.quinn.0, flat.0),
    );
    assert!(
        matches!(vf.ended(), Err(ContractError::Infeasible(_))),
        "a contract that is not a lease was allowed: {:?}",
        vf.ended()
    );
    assert_eq!(state(&tw), ContractState::Active);
    assert_eq!(pending(&tw), None, "a refused key began a transition");
    assert_eq!(
        snap(&tw).contract(plain).expect("the draft").state,
        ContractState::Draft
    );
    assert_eq!(obligation(&tw, rent).status, ObligationStatus::Active);
    assert_eq!(journal_len(&tw), posts, "a refused key posted something");
    assert_eq!(claims(&tw).len(), 1);

    // The owner's key: the plan first (under the deterministic key), then the state.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease {id}=#{} #{}", r.quinn.0, flat.0),
    );
    assert_eq!(vf.ended(), Ok(LeaseEnd::Ended));
    assert_eq!(state(&tw), ContractState::Cancelled);
    assert_eq!(pending(&tw), None);
    assert_eq!(
        obligation(&tw, rent).status,
        ObligationStatus::Cancelled,
        "the rent outlived the lease"
    );
    assert!(
        claims(&tw).is_empty(),
        "the lease claim stranded on the premises: {:?}",
        claims(&tw)
    );
    assert_eq!(journal_len(&tw), posts + 1);
    assert_eq!(
        refunds(&tw),
        1,
        "the unwind did not post under contract/{id}/refund/0"
    );
    assert_eq!(held(&tw, r.pat_acct), 0);
    // Two more rent periods fall due: nothing is charged.
    sweep_at(&mut tw, 200);
    sweep_at(&mut tw, 300);
    assert_eq!(bal(&tw, r.pat_acct), 900, "a cancelled lease charged rent");
    assert_eq!(bal(&tw, r.quinn_acct), 100);

    // Idempotent: a second end is `AlreadyEnded` and posts nothing.
    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease {id}=#{} #{}", r.quinn.0, flat.0),
    );
    assert_eq!(vf.ended(), Ok(LeaseEnd::AlreadyEnded));
    assert_eq!(journal_len(&tw), posts + 1);

    // Replay after a lost state write: the journal has the refund, the doc says
    // Active. The retried unwind finds nothing left to release (the claim is already
    // off) and lands the state with no second post.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &mush_economy::contract_key(id), |v| {
        v["state"] = serde_json::json!("Active");
        v["pending"] = serde_json::json!(null);
    });
    reboot(&mut tw, kv);
    assert!(!store(&tw).is_frozen());
    assert_eq!(
        state(&tw),
        ContractState::Active,
        "the patched doc did not load as Active"
    );
    assert_eq!(
        journal_len(&tw),
        posts + 1,
        "the journal did not survive the reboot"
    );
    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease {id}=#{} #{}", r.quinn.0, flat.0),
    );
    assert_eq!(vf.ended(), Ok(LeaseEnd::Ended));
    assert_eq!(state(&tw), ContractState::Cancelled);
    assert_eq!(
        journal_len(&tw),
        posts + 1,
        "the replay posted a second unwind"
    );
    assert_eq!(refunds(&tw), 1);
    assert_eq!(obligation(&tw, rent).status, ObligationStatus::Cancelled);
    sweep_at(&mut tw, 400);
    assert_eq!(bal(&tw, r.pat_acct), 900);
}

/// An `Offered` lease (the tenant never accepted) ends with an empty plan — nothing
/// was placed, so nothing posts — and a later acceptance is refused rather than
/// activating a lease whose tenancy is gone.
#[test]
fn lb_rent_after_end_end_lease_on_an_offered_lease_writes_the_state_alone() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 1_000);
    let flat = thing(&mut tw, "flat");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", flat.0, r.quinn.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vf/lease Lease=#{} #{} #{} {} {}",
            r.quinn.0, r.pat.0, flat.0, r.pat_acct, r.quinn_acct
        ),
    );
    let id = vf.lease().expect("the lease was offered");
    assert_eq!(
        snap(&tw).contract(id).expect("the lease").state,
        ContractState::Offered
    );
    let posts = journal_len(&tw);

    ok(
        &mut tw,
        GOD,
        &format!("@vf/endlease {id}=#{} #{}", r.quinn.0, flat.0),
    );
    assert_eq!(vf.ended(), Ok(LeaseEnd::Ended));
    assert_eq!(
        snap(&tw).contract(id).expect("the lease").state,
        ContractState::Cancelled
    );
    assert_eq!(
        journal_len(&tw),
        posts,
        "an offered lease's ending posted something"
    );
    err(&mut tw, r.pat, &format!("@contract/accept {id}"));
    assert!(
        snap(&tw)
            .obligations()
            .all(|o| o.cause != ObligationCause::Contract(id)),
        "a cancelled lease minted rent"
    );
    assert!(
        snap(&tw)
            .asset(flat)
            .expect("the flat")
            .encumbrances
            .is_empty()
    );
    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.pat_acct), 1_000);
}

// ======================================================================
// ---- SP1: declared cadences ----
// ======================================================================
//
// D-LBK-7 as decided (08h §2 Q6, K answer 4): a cadence named in
// `[economy.cadences]` is registered at composition as
// `ClockRef::Registered { owner: economy, id: cadence.<name>, generation }`; an
// existing document of that name is adopted at boot (schema 2, parameters kept); a
// missing one is created; a runtime cadence stays a legacy adapter and registers
// nothing. `EconomicClock` and the sweep are untouched.

fn declared(entries: &[(&str, DeclaredCadenceClock, i64)]) -> EconomyConfig {
    EconomyConfig {
        cadences: entries
            .iter()
            .map(|(name, clock, period_secs)| {
                (
                    SmolStr::new(name),
                    DeclaredCadence {
                        clock: *clock,
                        period_secs: *period_secs,
                        catch_up: mush_economy::DEFAULT_CATCH_UP,
                    },
                )
            })
            .collect(),
        ..EconomyConfig::default()
    }
}
fn declared_modules(cfg: EconomyConfig) -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![]).with_config(cfg)),
    ]
}
fn boot_declared(cfg: EconomyConfig) -> TestWorld {
    TestWorld::with_modules(declared_modules(cfg))
}
fn registered(name: &str, generation: u64) -> ClockRef {
    ClockRef::Registered {
        owner: OwnerId::new("economy").expect("the economy's owner id"),
        id: CanonicalId::new(format!("cadence.{name}")).expect("a canonical clock id"),
        generation,
    }
}
fn cadence_named(tw: &TestWorld, name: &str) -> mush_economy::CadenceClock {
    mush_economy::cadence_by_name(snap(tw), name)
        .cloned()
        .unwrap_or_else(|| panic!("no cadence named {name}"))
}
fn declared_pages(tw: &TestWorld, id: CadenceId) -> Vec<mush_economy::Escalation> {
    store(tw)
        .escalations()
        .filter(|e| e.subject == EscalationSubject::DeclaredCadence(id))
        .cloned()
        .collect()
}

/// A declared cadence is created at boot, stamped schema 2, and readable through
/// the kernel registry: `Engine::clock_now` answers its `position` under its
/// `generation`, the sweep still advances it from its own source, `revision` moves
/// without touching the registry's generation, and any other generation is stale.
#[test]
fn sp1_a_declared_cadence_is_registered_at_composition_and_read_through_the_registry() {
    let mut tw = boot_declared(declared(&[("season", DeclaredCadenceClock::RealUtc, 100)]));
    let c = cadence_named(&tw, "season");
    assert_eq!(
        (
            c.schema,
            c.source,
            c.period,
            c.catch_up_cap,
            c.position,
            c.generation
        ),
        (
            CADENCE_SCHEMA_DECLARED,
            CadenceSource::RealUtc,
            100,
            mush_economy::DEFAULT_CATCH_UP,
            0,
            mush_economy::CADENCE_GENERATION_INITIAL
        )
    );
    let stamp = tw
        .engine()
        .clock_now(&registered("season", 1))
        .expect("a declared cadence is readable through the registry");
    assert_eq!(stamp.at, 0);
    assert_eq!(stamp.clock, registered("season", 1));

    // The sweep advances it from its own source, exactly as before; the registry
    // reads the moved position.
    let t0 = tw.world().now();
    sweep_at(&mut tw, t0 + 250);
    let c = cadence_named(&tw, "season");
    assert_eq!(c.position, 2);
    assert_eq!(
        tw.engine()
            .clock_now(&registered("season", 1))
            .expect("still readable")
            .at,
        2
    );
    // `revision` is the staleness stamp and moved; `generation` did not (08h Q6), and
    // the registry keys on the generation alone.
    assert_eq!((c.revision, c.generation), (2, 1));
    let stale = tw
        .engine()
        .clock_now(&registered("season", 2))
        .expect_err("generation 2 does not exist");
    assert!(
        matches!(
            stale,
            ClockReadError::Provider {
                source: ClockProviderFailure::StaleGeneration {
                    requested: 2,
                    current: 1
                },
                ..
            }
        ),
        "{stale:?}"
    );
    // The legacy adapter resolves the same clock, untouched.
    assert_eq!(
        Clocks::new(0, 0).now_on(EconomicClock::Cadence(c.id), snap(&tw), 0),
        Some(2)
    );
    let seen = ok(&mut tw, GOD, &format!("@econ/cadence show {}", c.id));
    assert!(seen.contains("declared"), "{seen}");
    assert!(seen.contains("generation 1 · schema 2"), "{seen}");

    // A reboot over its own documents adopts silently: same clock, same position,
    // nothing paged.
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    let c = cadence_named(&tw, "season");
    assert_eq!((c.schema, c.position), (CADENCE_SCHEMA_DECLARED, 2));
    assert_eq!(snap(&tw).cadences().count(), 1, "a second clock was minted");
    assert!(declared_pages(&tw, c.id).is_empty());
    assert_eq!(
        tw.engine()
            .clock_now(&registered("season", 1))
            .expect("readable after the reboot")
            .at,
        2
    );
}

/// An existing document with a declared name is ADOPTED: stamped schema 2, its
/// parameters left as persisted (the configuration never rewrites a live clock), and
/// a clock/period disagreement is paged exactly once through the ladder however many
/// boots see it.
#[test]
fn sp1_adoption_stamps_schema_2_keeps_the_persisted_parameters_and_pages_a_mismatch_once() {
    let mut first = boot();
    ok(
        &mut first,
        GOD,
        "@econ/cadence create season source=real:100",
    );
    let mut kv = fold_kv(&mut first);

    // The declaration disagrees on both counts.
    let mut tw = boot_declared(declared(&[("season", DeclaredCadenceClock::WorldIc, 200)]));
    let _ = tw.take_module_ops(); // the fresh boot's own creation, not under test
    reboot(&mut tw, kv.clone());
    let c = cadence_named(&tw, "season");
    assert_eq!(c.id, CadenceId(0));
    assert_eq!(
        (c.schema, c.source, c.period, c.catch_up_cap),
        (
            CADENCE_SCHEMA_DECLARED,
            CadenceSource::RealUtc,
            100,
            mush_economy::DEFAULT_CATCH_UP
        ),
        "adoption rewrote a live clock"
    );
    assert_eq!(
        snap(&tw).cadences().count(),
        1,
        "a second `season` was minted"
    );
    let pages = declared_pages(&tw, CadenceId(0));
    assert_eq!(pages.len(), 1, "{pages:?}");
    let page = &pages[0];
    assert_eq!(page.kind, EscalationKind::BootReconciliation);
    assert!(page.is_open());
    assert!(
        page.context.contains("economy.cadence.declared:season"),
        "{}",
        page.context
    );
    assert!(
        page.context.contains("\"world_ic\"") && page.context.contains("\"real_utc\""),
        "{}",
        page.context
    );
    assert!(
        page.recommended
            .iter()
            .any(|r| r.contains("@econ/cadence show #0")),
        "{:?}",
        page.recommended
    );
    // The registry reads the adopted document.
    assert_eq!(
        tw.engine()
            .clock_now(&registered("season", 1))
            .expect("readable")
            .at,
        0
    );
    // The adoption and the page are durable, and a second boot over them pages
    // nothing new and mints nothing.
    merge(&mut kv, drain(&mut tw));
    assert_eq!(value_of(&kv, "cadence:0")["schema"], serde_json::json!(2));
    assert_eq!(value_of(&kv, "cadence:0")["period"], serde_json::json!(100));
    reboot(&mut tw, kv.clone());
    assert_eq!(declared_pages(&tw, CadenceId(0)).len(), 1);
    assert_eq!(snap(&tw).cadences().count(), 1);
    let seen = ok(&mut tw, GOD, "@econ/cadence show 0");
    assert!(seen.contains("declared"), "{seen}");
    assert!(seen.contains("every 100s real"), "{seen}");
    let seen = ok(&mut tw, GOD, "@econ/alerts");
    assert!(seen.contains("declared cadence #0"), "{seen}");
    // Acknowledging changes neither side.
    let seen = ok(&mut tw, GOD, "@econ/ack 0");
    assert!(seen.contains("does not rewrite a live clock"), "{seen}");
    let c = cadence_named(&tw, "season");
    assert_eq!((c.source, c.period), (CadenceSource::RealUtc, 100));
}

/// A cadence created at runtime by `@econ/cadence create` is NOT registered: the
/// registry has no provider for it, it keeps the legacy schema, `show` says so, and
/// the legacy adapter still resolves it.
#[test]
fn sp1_a_runtime_cadence_is_not_registered() {
    let mut tw = boot_declared(declared(&[("season", DeclaredCadenceClock::RealUtc, 100)]));
    ok(&mut tw, GOD, "@econ/cadence create ad_hoc source=real:10");
    let c = cadence_named(&tw, "ad_hoc");
    assert_eq!(c.schema, mush_economy::CADENCE_SCHEMA_LEGACY);
    let missing = tw
        .engine()
        .clock_now(&registered("ad_hoc", 1))
        .expect_err("a runtime cadence has no provider");
    assert!(
        matches!(missing, ClockReadError::ProviderMissing { .. }),
        "{missing:?}"
    );
    let seen = ok(&mut tw, GOD, &format!("@econ/cadence show {}", c.id));
    assert!(seen.contains("runtime"), "{seen}");
    assert!(!seen.contains("declared"), "{seen}");
    assert_eq!(
        Clocks::new(0, 0).now_on(EconomicClock::Cadence(c.id), snap(&tw), 0),
        Some(0)
    );
    // The declared one beside it is unaffected.
    assert!(tw.engine().clock_now(&registered("season", 1)).is_ok());
}

/// `[economy.cadences]` is refused whole — at the typed config and again at
/// composition — for a 33rd clock, a name that is not a cadence name and a canonical
/// clock id, a non-positive period, or a catch-up outside the band.
#[test]
fn sp1_configuration_refuses_33_cadences_and_a_bad_name() {
    let many: Vec<(String, DeclaredCadenceClock, i64)> = (0..33)
        .map(|i| (format!("c{i}"), DeclaredCadenceClock::RealUtc, 100))
        .collect();
    let refs: Vec<(&str, DeclaredCadenceClock, i64)> =
        many.iter().map(|(n, c, p)| (n.as_str(), *c, *p)).collect();
    let cfg = declared(&refs);
    assert_eq!(
        cfg.check_declared_cadences(),
        Err(DeclaredCadenceError::TooMany {
            count: 33,
            max: mush_economy::MAX_DECLARED_CADENCES
        })
    );
    // Composition refuses it too, with the reason.
    let (world, registry) = mush_testkit::boot_world();
    let refused = Engine::with_modules(world, registry, declared_modules(cfg))
        .err()
        .expect("composition refused the list");
    assert!(refused.to_string().contains("33"), "{refused}");
    // Thirty-two is the bound, not thirty-one.
    assert_eq!(declared(&refs[..32]).check_declared_cadences(), Ok(()));

    let long = "x".repeat(33);
    for bad in [
        "Season",
        "a..b",
        ".season",
        "season.",
        "sea son",
        "sea-son",
        "",
        long.as_str(),
    ] {
        let cfg = declared(&[(bad, DeclaredCadenceClock::RealUtc, 100)]);
        assert!(
            matches!(
                cfg.check_declared_cadences(),
                Err(DeclaredCadenceError::Name { .. })
            ),
            "{bad:?} was accepted"
        );
        let (world, registry) = mush_testkit::boot_world();
        assert!(
            Engine::with_modules(world, registry, declared_modules(cfg)).is_err(),
            "{bad:?} composed"
        );
    }
    // A dotted, 32-byte name is a name.
    assert_eq!(
        declared(&[("chronicle.period", DeclaredCadenceClock::WorldIc, 1)])
            .check_declared_cadences(),
        Ok(())
    );
    assert_eq!(
        declared(&[("x".repeat(32).as_str(), DeclaredCadenceClock::WorldIc, 1)])
            .check_declared_cadences(),
        Ok(())
    );
    assert!(matches!(
        declared(&[("season", DeclaredCadenceClock::WorldIc, 0)]).check_declared_cadences(),
        Err(DeclaredCadenceError::Period { .. })
    ));
    for catch_up in [0, mush_economy::MAX_CATCH_UP_CAP + 1] {
        let mut cfg = declared(&[("season", DeclaredCadenceClock::WorldIc, 1)]);
        cfg.cadences.get_mut("season").expect("the stanza").catch_up = catch_up;
        assert!(
            matches!(
                cfg.check_declared_cadences(),
                Err(DeclaredCadenceError::CatchUp { .. })
            ),
            "catch_up {catch_up} was accepted"
        );
    }
}

/// An old-shape document (no `generation`, no `schema`) with a declared name loads,
/// is adopted to schema 2 at generation 1, and — its parameters matching — pages
/// nothing.
#[test]
fn sp1_an_old_shape_document_with_a_declared_name_loads_and_is_adopted() {
    let mut first = boot();
    ok(
        &mut first,
        GOD,
        "@econ/cadence create season source=real:100",
    );
    ok(&mut first, GOD, "@econ/cadence create other source=manual");
    let mut kv = fold_kv(&mut first);
    patch(&mut kv, "cadence:0", |v| {
        v.as_object_mut().expect("an object").remove("generation");
        v.as_object_mut().expect("an object").remove("schema");
    });
    let mut tw = boot_declared(declared(&[("season", DeclaredCadenceClock::RealUtc, 100)]));
    let _ = tw.take_module_ops();
    reboot(&mut tw, kv);
    let c = cadence_named(&tw, "season");
    assert_eq!(
        (c.id, c.generation, c.schema, c.revision),
        (
            CadenceId(0),
            mush_economy::CADENCE_GENERATION_INITIAL,
            CADENCE_SCHEMA_DECLARED,
            1
        )
    );
    assert!(declared_pages(&tw, CadenceId(0)).is_empty());
    assert_eq!(
        tw.engine()
            .clock_now(&registered("season", 1))
            .expect("readable")
            .at,
        0
    );
    // The runtime clock beside it is left exactly as it was.
    let other = cadence_named(&tw, "other");
    assert_eq!(other.schema, mush_economy::CADENCE_SCHEMA_LEGACY);
    assert!(matches!(
        tw.engine().clock_now(&registered("other", 1)),
        Err(ClockReadError::ProviderMissing { .. })
    ));
}

// ======================================================================
// ---- E6a step 1: S3 — player-debit obligations for rulesets ----
//
// `create_obligation` / `cancel_obligation` / `reinstate_obligation` (08f §2
// item 3): a ruleset creates a recurring debit under the ACTOR's own debit
// authority — never a `System` debtor — and touches only what its namespace
// created. Driven through the `@vf` probe, because the seams are crate APIs and
// deliberately not verbs.
// ======================================================================

/// A 50-per-100-seconds upkeep from `debit` to `credit`, first due at 100, under
/// `namespace`.
fn upkeep(debit: AccountId, credit: AccountId, namespace: &str, grace: u8) -> ObligationSpec {
    ObligationSpec {
        debit_account: debit,
        credit_account: credit,
        amount: Amount::from_minor(50),
        schedule: EconomicSchedule::new(EconomicClock::RealUtc, 100, Some(100), 3),
        grace: GracePolicy::new(grace),
        notify_on_success: false,
        cause: ObligationCause::Ruleset {
            namespace: namespace.into(),
        },
    }
}

#[test]
fn e6s1_s3_a_player_creates_a_debit_on_their_own_wallet_and_the_sweep_collects_it() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 500);
    vf.stage_create(upkeep(r.pat_acct, r.quinn_acct, "probe.upkeep", 1));
    ok(&mut tw, r.pat, "@vf/create");
    let id = vf.created().expect("the seam refused a player's own debit");
    let o = obligation(&tw, id);
    assert_eq!(o.debtor, mush_org::PrincipalId::Object(r.pat));
    assert_eq!(o.creditor, mush_org::PrincipalId::Object(r.quinn));
    assert_eq!(
        o.cause,
        ObligationCause::Ruleset {
            namespace: "probe.upkeep".into()
        }
    );
    assert_eq!(o.minted_by, None, "a player debit is not a mint");
    assert_eq!(o.status, ObligationStatus::Active);

    // The sweep collects it like any obligation: `ObligationAuth` permits a
    // non-`System` debit on the record's own account.
    sweep_at(&mut tw, 150);
    assert_eq!(
        bal(&tw, r.pat_acct),
        450,
        "the sweep did not collect the period"
    );
    assert_eq!(bal(&tw, r.quinn_acct), 50);
    assert_eq!(obligation(&tw, id).settled_total, 1);

    // Boot loads it, and the next period collects after the restart.
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert!(!store(&tw).is_frozen());
    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active);
    assert_eq!(o.minted_by, None);
    sweep_at(&mut tw, 250);
    assert_eq!(
        bal(&tw, r.pat_acct),
        400,
        "the reloaded record did not collect"
    );
}

#[test]
fn e6s1_s3_a_debit_on_a_wallet_the_actor_may_not_debit_is_refused() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 500);
    // Quinn cannot put a standing charge on Pat's wallet.
    vf.stage_create(upkeep(r.pat_acct, r.quinn_acct, "probe.upkeep", 1));
    ok(&mut tw, r.quinn, "@vf/create");
    assert_eq!(vf.created(), Err(EconomyError::Unauthorized));
    assert_eq!(
        snap(&tw).obligations().count(),
        0,
        "a refused create wrote a record"
    );
    // A non-`Ruleset` cause is not this seam's to create, whoever asks.
    let mut staffish = upkeep(r.pat_acct, r.quinn_acct, "probe.upkeep", 1);
    staffish.cause = ObligationCause::Staff {
        reason: "probe".into(),
    };
    vf.stage_create(staffish);
    ok(&mut tw, GOD, "@vf/create");
    assert_eq!(vf.created(), Err(EconomyError::Unauthorized));
    assert_eq!(snap(&tw).obligations().count(), 0);
    // Staff may create one on anybody's wallet — the approver at chargen.
    vf.stage_create(upkeep(r.pat_acct, r.quinn_acct, "probe.upkeep", 1));
    ok(&mut tw, GOD, "@vf/create");
    vf.created()
        .expect("staff hold debit authority on every wallet");
}

#[test]
fn e6s1_s3_a_system_debtor_is_refused_even_for_staff() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 0);
    vf.stage_create(upkeep(r.faucet, r.quinn_acct, "probe.upkeep", 1));
    ok(&mut tw, GOD, "@vf/create");
    assert_eq!(
        vf.created(),
        Err(EconomyError::Unauthorized),
        "a faucet debit is a standing mint and has exactly one route (create_system_obligation)"
    );
    assert_eq!(snap(&tw).obligations().count(), 0);
}

#[test]
fn e6s1_s3_cancel_and_reinstate_are_locked_to_the_creating_namespace() {
    let (mut tw, vf) = boot_probe();
    let r = rig(&mut tw, 0);
    // Grace 0 and no funds: the first miss is delinquency.
    vf.stage_create(upkeep(r.pat_acct, r.quinn_acct, "probe.upkeep", 0));
    ok(&mut tw, r.pat, "@vf/create");
    let id = vf.created().expect("created");
    sweep_at(&mut tw, 150);
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);

    // Another namespace may neither reinstate nor cancel it.
    ok(&mut tw, GOD, &format!("@vf/restore {id}=other.ns"));
    assert_eq!(vf.ruleset(), Err(EconomyError::Unauthorized));
    ok(&mut tw, GOD, &format!("@vf/cancelob {id}=other.ns"));
    assert_eq!(vf.ruleset(), Err(EconomyError::Unauthorized));
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);
    // A record nobody's namespace created — a staff obligation — is nobody's either.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100 start=100",
            r.quinn_acct, r.pat_acct
        ),
    );
    let staffed = snap(&tw)
        .obligations()
        .map(|o| o.id)
        .max()
        .expect("the staff record");
    assert_ne!(staffed, id);
    ok(
        &mut tw,
        GOD,
        &format!("@vf/cancelob {staffed}=probe.upkeep"),
    );
    assert_eq!(vf.ruleset(), Err(EconomyError::Unauthorized));

    // The creating namespace reinstates it: back in the pool, re-anchored, and the
    // delinquent periods are not charged.
    ok(&mut tw, GOD, &format!("@vf/restore {id}=probe.upkeep"));
    assert_eq!(vf.ruleset(), Ok(true));
    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active);
    assert!(
        o.schedule.next_due > 150,
        "the schedule was not re-anchored"
    );
    // Nothing to reinstate the second time.
    ok(&mut tw, GOD, &format!("@vf/restore {id}=probe.upkeep"));
    assert_eq!(vf.ruleset(), Ok(false));

    // And the creating namespace cancels it; a cancelled record is terminal to
    // both seams.
    ok(&mut tw, GOD, &format!("@vf/cancelob {id}=probe.upkeep"));
    assert_eq!(vf.ruleset(), Ok(false));
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Cancelled);
    ok(&mut tw, GOD, &format!("@vf/cancelob {id}=probe.upkeep"));
    assert_eq!(vf.ruleset(), Err(EconomyError::ObligationTerminal(id)));
    ok(&mut tw, GOD, &format!("@vf/restore {id}=probe.upkeep"));
    assert_eq!(vf.ruleset(), Err(EconomyError::ObligationTerminal(id)));
}

// ======================================================================
// ---- E6a step 1: S5 — `InstrumentDisplay` + one `render_amount` ----
//
// E2-C16 / 08g §12.1 item 4: an instrument may declare a symbol and where it
// goes, and every player-facing amount goes through one scale-aware,
// symbol-aware render. The look-back found five renders printing raw minor units
// (08g §3); the economy's four are pinned here and in `e4b2.rs` by message, and
// the fifth (housing's `Property.upkeep` line) was removed with E5b-D34's fold.
// ======================================================================

/// Two displayed instruments — `yen` (scale 0, `¥` prefix) and `cent` (scale 2,
/// `¢` suffix) — with a faucet each and a funded wallet apiece.
struct Displayed {
    pat: ObjId,
    quinn: ObjId,
    yen_faucet: AccountId,
    pat_yen: AccountId,
    quinn_yen: AccountId,
    quinn_cent: AccountId,
}

fn displayed(tw: &mut TestWorld) -> Displayed {
    let pat = tw.spawn_player("Pat");
    let quinn = tw.spawn_player("Quinn");
    let seen = ok(tw, GOD, "@instrument/register yen=0 display=¥:prefix");
    assert!(seen.contains("renders as ¥<amount>"), "{seen}");
    let seen = ok(tw, GOD, "@instrument/register cent=2 display=¢:suffix");
    assert!(seen.contains("renders as <amount>¢"), "{seen}");
    ok(tw, GOD, "@account/system yenfaucet=yen");
    ok(tw, GOD, "@account/system centfaucet=cent");
    ok(tw, GOD, &format!("@account/open #{}=yen", pat.0));
    ok(tw, GOD, &format!("@account/open #{}=yen", quinn.0));
    ok(tw, GOD, &format!("@account/open #{}=cent", quinn.0));
    let find = |tw: &TestWorld, owner: mush_org::PrincipalId, code: &str| -> AccountId {
        snap(tw)
            .accounts()
            .find(|a| a.owner == owner && a.instrument.as_str() == code)
            .expect("account")
            .id
    };
    let yen_faucet = find(
        tw,
        mush_org::PrincipalId::System(mush_org::SystemPrincipalId::new("yenfaucet").expect("id")),
        "yen",
    );
    let cent_faucet = find(
        tw,
        mush_org::PrincipalId::System(mush_org::SystemPrincipalId::new("centfaucet").expect("id")),
        "cent",
    );
    let pat_yen = find(tw, mush_org::PrincipalId::Object(pat), "yen");
    let quinn_yen = find(tw, mush_org::PrincipalId::Object(quinn), "yen");
    let quinn_cent = find(tw, mush_org::PrincipalId::Object(quinn), "cent");
    ok(tw, GOD, &format!("@econ/mint {yen_faucet}={pat_yen} 1500"));
    ok(
        tw,
        GOD,
        &format!("@econ/mint {cent_faucet}={quinn_cent} 1250"),
    );
    Displayed {
        pat,
        quinn,
        yen_faucet,
        pat_yen,
        quinn_yen,
        quinn_cent,
    }
}

#[test]
fn e6s1_s5_a_prefix_and_a_suffix_instrument_render_at_their_own_scale_everywhere() {
    let mut tw = boot();
    let d = displayed(&mut tw);
    // Scale 0 with a prefix symbol; scale 2 with a suffix symbol.
    let seen = ok(&mut tw, d.pat, &format!("@account/show {}", d.pat_yen));
    assert!(seen.contains("balance ¥1500"), "{seen}");
    let seen = ok(&mut tw, d.quinn, &format!("@account/show {}", d.quinn_cent));
    assert!(seen.contains("balance 12.50¢"), "{seen}");
    let seen = ok(&mut tw, GOD, "@account/list");
    assert!(
        seen.contains("= ¥1500 [") && seen.contains("= 12.50¢ ["),
        "{seen}"
    );
    // A negative line renders with the sign before a prefix symbol.
    ok(
        &mut tw,
        d.pat,
        &format!("@econ/pay {}={} 500", d.pat_yen, d.quinn_yen),
    );
    let seen = ok(&mut tw, d.pat, &format!("@econ/history {}", d.pat_yen));
    assert!(seen.contains("-¥500"), "{seen}");
    // A reservation, a rule, and a standing charge all say ¥.
    ok(
        &mut tw,
        d.pat,
        &format!("@hold/place {}=100 a probe", d.pat_yen),
    );
    let seen = ok(&mut tw, d.pat, "@hold/list");
    assert!(seen.contains("¥100 [held]"), "{seen}");
    let hold = only_hold(&tw);
    let seen = ok(&mut tw, d.pat, &format!("@hold/show #{hold}"));
    assert!(seen.contains("— ¥100 [held]"), "{seen}");
    let seen = ok(
        &mut tw,
        GOD,
        &format!(
            "@account/approval {}=100 threshold:1 window=400/3600",
            d.pat_yen
        ),
    );
    assert!(
        seen.contains("above ¥100") && seen.contains("over ¥400"),
        "{seen}"
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 50 every=100 start=100",
            d.pat_yen, d.quinn_yen
        ),
    );
    let id = only_obligation(&tw);
    let seen = ok(&mut tw, d.pat, &format!("@obligation/show #{id}"));
    assert!(seen.contains("¥50 ["), "{seen}");
    let seen = ok(&mut tw, d.pat, "@obligation/list");
    assert!(seen.contains("¥50 ["), "{seen}");
    // A lien renders in the instrument it names — one of 08g §3's raw-minor sites.
    let van = thing(&mut tw, "van");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", van.0, d.pat.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@asset/lien #{}=#{} 1250 cent", van.0, d.quinn.0),
    );
    let seen = ok(&mut tw, GOD, &format!("@asset/show #{}", van.0));
    assert!(seen.contains("lien 12.50¢ cent to"), "{seen}");
    // The display survives a reboot, and a malformed one refuses to load.
    let mut kv = fold_kv(&mut tw);
    reboot(&mut tw, kv.clone());
    let seen = ok(&mut tw, d.pat, &format!("@account/show {}", d.pat_yen));
    assert!(seen.contains("balance ¥1000"), "{seen}");
    patch(&mut kv, "instrument:yen", |v| {
        v["display"] =
            serde_json::json!({ "symbol": "a symbol far too long", "position": "Prefix" });
    });
    let stored: Result<mush_economy::InstrumentDef, _> =
        serde_json::from_value(value_of(&kv, "instrument:yen"));
    assert!(stored.is_err(), "an oversized symbol deserialized");
    let _ = d.yen_faucet;
}

#[test]
fn e6s1_s5_the_mint_cap_notice_renders_the_cap_at_scale() {
    // One of 08g §3's raw-minor sites: the sweep's over-cap page printed the cap in
    // minor units. It now renders in the debit account's instrument.
    let (mut tw, vf) = boot_probe();
    let d = displayed(&mut tw);
    vf.stage_mint(
        "yenfaucet",
        ObligationSpec {
            debit_account: d.yen_faucet,
            credit_account: d.quinn_yen,
            amount: Amount::from_minor(100),
            schedule: EconomicSchedule::new(EconomicClock::RealUtc, 100, Some(100), 3),
            grace: GracePolicy::default(),
            notify_on_success: false,
            cause: ObligationCause::Ruleset {
                namespace: "probe.income".into(),
            },
        },
    );
    ok(&mut tw, GOD, "@vf/mint");
    let id = vf.minted().expect("an under-cap income line was refused");
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("obligation:{}", id.0), |v| {
        v["amount"] = serde_json::json!(60_000);
    });
    reboot(&mut tw, kv);
    sweep_at(&mut tw, 200);
    let record = alert_for(&tw, EscalationSubject::Obligation(id));
    assert!(
        record.context.contains("mint cap (¥50000)"),
        "the cap did not render at scale with its symbol: {}",
        record.context
    );
}

// ======================================================================
// ---- E6a step 1 verify fold ----
//
// The adversarial verify's confirmed findings that land on the ladder, the
// obligation seams and the declared clocks.
// ======================================================================

#[test]
fn vf_m3_the_ruleset_obligation_seams_ask_who_is_asking() {
    let (mut tw, vf) = boot_probe();
    // An empty wallet, so the record misses its periods and goes delinquent — which
    // is the only state `reinstate_obligation` acts on.
    let r = rig(&mut tw, 0);
    let stranger = tw.spawn_player("Stranger");
    vf.stage_create(upkeep(r.pat_acct, r.quinn_acct, "probe.upkeep", 0));
    ok(&mut tw, r.pat, "@vf/create");
    let id = vf.created().expect("the debtor's own upkeep was refused");

    // The namespace says WHICH records this seam may touch. It said nothing about
    // WHO is asking, and `Ctx::actor` is whoever typed the command that reached the
    // ruleset — so a `+lifestyle` verb wired to this seam let any player cancel any
    // lifestyle obligation in the game, their own or a stranger's.
    ok(
        &mut tw,
        stranger,
        &format!("@vf/cancelob {id}=probe.upkeep"),
    );
    assert_eq!(
        vf.ruleset(),
        Err(EconomyError::Unauthorized),
        "a stranger cancelled somebody else's standing debt"
    );
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Active);

    // Make it delinquent, so reinstating is a real act with a real consequence:
    // reinstating **re-anchors the schedule**, so the periods it spent delinquent
    // are never charged (D-E6-23). Forgiving somebody else's arrears is exactly the
    // thing a namespace-only gate let a stranger do.
    sweep_at(&mut tw, 50);
    sweep_at(&mut tw, 150);
    sweep_at(&mut tw, 250);
    assert_eq!(
        obligation(&tw, id).status,
        ObligationStatus::Delinquent,
        "the rig did not reach the state reinstate is about"
    );
    ok(&mut tw, stranger, &format!("@vf/restore {id}=probe.upkeep"));
    assert_eq!(
        vf.ruleset(),
        Err(EconomyError::Unauthorized),
        "a stranger forgave somebody else's arrears"
    );
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);

    // The debtor may: it is their wallet the debit comes out of.
    ok(&mut tw, r.pat, &format!("@vf/restore {id}=probe.upkeep"));
    assert_eq!(vf.ruleset(), Ok(true), "the debtor could not reinstate");
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Active);

    // ...and so may staff, who hold debit authority on every wallet.
    ok(&mut tw, GOD, &format!("@vf/cancelob {id}=probe.upkeep"));
    assert_eq!(vf.ruleset(), Ok(false), "staff could not cancel");
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Cancelled);
}

#[test]
fn vf_r8_a_dropped_announcement_does_not_suppress_the_freeze_record() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let _ = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);

    // A page the observer chain dropped used to mint an `Economy`-subject record —
    // the same subject the whole-economy freeze uses — and `open_freeze_record`
    // returns early while ANY `Economy` record stands, acknowledged or not. So a lost
    // chronicle line, which is not about the journal at all, could silently suppress
    // the one record that says the economy has stopped.
    assert_ne!(
        EscalationSubject::DroppedPage,
        EscalationSubject::Economy,
        "a dropped page still shares the freeze record's subject"
    );
    let dropped = mush_economy::Escalation::page(
        mush_economy::EscalationId(900),
        EscalationKind::SettlementRefused,
        EscalationSubject::DroppedPage,
        "the observer chain hit its depth cap",
        vec!["@econ/alerts".to_string()],
        10,
    );
    kv.push((
        mush_economy::escalation_key(dropped.id),
        serde_json::to_value(&dropped).expect("an escalation serializes"),
    ));
    patch(&mut kv, "meta", |v| {
        v["frozen"] = serde_json::json!(true);
    });
    reboot(&mut tw, kv);
    assert!(store(&tw).is_frozen());
    assert_eq!(
        store(&tw)
            .escalations()
            .filter(|e| e.subject == EscalationSubject::DroppedPage)
            .count(),
        1,
        "the dropped-page record did not load"
    );

    sweep_at(&mut tw, 100);
    let economy: Vec<_> = store(&tw)
        .escalations()
        .filter(|e| e.subject == EscalationSubject::Economy)
        .cloned()
        .collect();
    assert_eq!(
        economy.len(),
        1,
        "a dropped announcement suppressed the whole-economy freeze record"
    );
    assert_eq!(economy[0].kind, EscalationKind::EconomyFrozen);
    assert!(
        economy[0]
            .recommended
            .iter()
            .any(|c| c.contains("@econ/unfreeze")),
        "the freeze record lost its recovery command: {:?}",
        economy[0].recommended
    );
}

#[test]
fn vf_r10_a_cadence_no_longer_declared_goes_back_to_the_legacy_schema() {
    // A world that declares `campaign` in `[economy.cadences]`...
    let declared = mush_economy::EconomyConfig {
        cadences: [(
            SmolStr::new_static("campaign"),
            mush_economy::DeclaredCadence {
                clock: mush_economy::DeclaredCadenceClock::RealUtc,
                period_secs: 3_600,
                catch_up: mush_economy::DEFAULT_CATCH_UP,
            },
        )]
        .into_iter()
        .collect(),
        ..mush_economy::EconomyConfig::default()
    };
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![]).with_config(declared)),
    ]);
    let cadence = snap(&tw)
        .cadences()
        .find(|c| c.name == "campaign")
        .cloned()
        .expect("the declared cadence was not created");
    assert_eq!(cadence.schema, mush_economy::CADENCE_SCHEMA_DECLARED);
    let kv = fold_kv(&mut tw);

    // ...and the stanza is removed. K's clock registry freezes at composition, so
    // nothing answers for `cadence.campaign` any more — but the record kept saying
    // "declared", which is the reading a consumer would trust and then get
    // `Unavailable` from.
    let mut plain = boot();
    reboot(&mut plain, kv);
    let after = snap(&plain)
        .cadences()
        .find(|c| c.name == "campaign")
        .cloned()
        .expect("the clock itself was removed");
    assert_eq!(
        after.schema,
        mush_economy::CADENCE_SCHEMA_LEGACY,
        "an undeclared cadence still shows as declared with no provider behind it"
    );
    // The clock itself is untouched: configuration never rewrites a running clock,
    // in either direction.
    assert_eq!(after.period, cadence.period);
    assert_eq!(after.source, cadence.source);
    assert_eq!(after.position, cadence.position);
    assert_eq!(after.generation, cadence.generation);
}

// ======================================================================
// ---- doc 18 cut IC6: a live lease is never "stale" ----
// ======================================================================

/// Doc 18 §4.1 / cut IC6: a live lease has no deadline and no condition progress by
/// design — it ends when somebody says it ends — so `page_stale_contracts` exempts an
/// `Active` lease-shaped contract. Swept past the configured age and again eight days
/// on, it opens no escalation whose subject is the lease, and it is still `Active`.
/// (`e4c.rs`'s `e6s1_s7_a_deadline_less_live_contract_pages_at_the_configured_age` is
/// the control: a live non-lease still pages.)
#[test]
fn ic6_a_live_lease_is_never_paged_as_stale() {
    let (mut tw, vf) = boot_probe_with(EconomyConfig {
        stale_contract_age_secs: 3_600,
        ..Default::default()
    });
    let now = 1_000;
    tw.set_now(now);
    let r = rig(&mut tw, 10_000_000);
    let flat = thing(&mut tw, "flat");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", flat.0, r.quinn.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vf/lease Lease=#{} #{} #{} {} {}",
            r.quinn.0, r.pat.0, flat.0, r.pat_acct, r.quinn_acct
        ),
    );
    let id = vf.lease().expect("the lease was offered");
    ok(&mut tw, r.pat, &format!("@contract/accept {id}"));
    assert_eq!(
        snap(&tw).contract(id).expect("the lease").state,
        ContractState::Active
    );
    for t in [now + 3_600 + 1_000, now + 8 * 86_400] {
        sweep_at(&mut tw, t);
        let paged = store(&tw)
            .escalations()
            .filter(|e| e.subject == EscalationSubject::Contract(id))
            .count();
        assert_eq!(paged, 0, "a live lease was paged as stale at {t}");
        assert_eq!(
            snap(&tw).contract(id).expect("the lease").state,
            ContractState::Active,
            "the lease did not stay live at {t}"
        );
    }
}
