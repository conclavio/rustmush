//! E2b acceptance: the EconomyModule attaches the pure E2a engine to a real world —
//! staff instrument/account setup, minting + transfer + reversal through the
//! validated post seam, per-record persistence, and the money-safety boot: full
//! journal re-derivation, directional self-heal of a lagging projection, and a
//! freeze on drift / a conservation break. Driven through the real Engine via
//! `TestWorld`.

use std::collections::HashMap;

use mush_core::{
    Audience, Effects, GOD, LedgerChunk, LedgerEntry, ModuleBootData, ModuleWrite, ObjId, Outbox,
    WorldEvent,
};
use mush_economy::{AccountId, EconomyStore};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "economy";

fn boot() -> TestWorld {
    // Economy authorizes through org; load both.
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::new()),
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
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().expect("economy store")
}
fn bal(tw: &TestWorld, id: u64) -> i64 {
    store(tw).snapshot().account(AccountId(id)).expect("account").posted_balance.minor()
}
fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
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
fn reboot(kv: Vec<(SmolStr, serde_json::Value)>) -> TestWorld {
    let mut tw = boot();
    tw.modules_loaded(HashMap::from([(SmolStr::new(NS), ModuleBootData { kv, streams: HashMap::new() })]));
    tw
}
fn try_boot(tw: &mut TestWorld, kv: Vec<(SmolStr, serde_json::Value)>) -> Result<(), String> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .modules_loaded(
            HashMap::from([(SmolStr::new(NS), ModuleBootData { kv, streams: HashMap::new() })]),
            &mut out,
            &mut fx,
        )
        .map_err(|e| format!("{e:?}"))
}
/// Register gold, a system faucet (acct 0), and one player account (acct 1); mint
/// `amount` into the player account. Returns the player object.
fn setup_mint(tw: &mut TestWorld, amount: i64) -> ObjId {
    assert!(!has_error(&tw.run(GOD, "@instrument/register gold=2")));
    let p = tw.spawn_player("Pat");
    assert!(!has_error(&tw.run(GOD, "@account/system faucet=gold")), "system account");
    assert!(!has_error(&tw.run(GOD, &format!("@account/open #{}=gold", p.0))), "player account");
    let ev = tw.run(GOD, &format!("@econ/mint 0=1 {amount}"));
    assert!(!has_error(&ev), "mint refused: {}", texts(&ev));
    p
}

// ----------------------------------------------------------------------

#[test]
fn mint_transfer_and_reverse_move_balances() {
    let mut tw = boot();
    let _p = setup_mint(&mut tw, 1000);
    assert_eq!(bal(&tw, 0), -1000, "faucet drives negative");
    assert_eq!(bal(&tw, 1), 1000, "player funded");
    // Open a second player account (acct 2) and transfer.
    let q = tw.spawn_player("Quinn");
    tw.run(GOD, &format!("@account/open #{}=gold", q.0));
    assert!(!has_error(&tw.run(GOD, "@econ/pay 1=2 300")), "transfer");
    assert_eq!(bal(&tw, 1), 700);
    assert_eq!(bal(&tw, 2), 300);
    // The transfer was transaction #1 (mint was #0); reverse it.
    assert!(!has_error(&tw.run(GOD, "@econ/reverse 1")), "reverse");
    assert_eq!(bal(&tw, 1), 1000, "transfer compensated");
    assert_eq!(bal(&tw, 2), 0);
    // Conservation across all gold accounts is zero.
    let sum: i128 = store(&tw).snapshot().accounts().map(|a| a.posted_balance.widen()).sum();
    assert_eq!(sum, 0);
}

#[test]
fn a_post_persists_the_bundle_and_survives_reboot() {
    let mut tw = boot();
    setup_mint(&mut tw, 500);
    // The mint persisted the journal chunk + both account docs + meta.
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(|o| o.ns == NS && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("log:journal:"))), "journal chunk persisted");
    assert!(ops.iter().any(|o| o.ns == NS && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == "account:1")), "account persisted");
    // Redo to capture the folded kv (take_module_ops above drained it).
    let mut tw = boot();
    setup_mint(&mut tw, 500);
    let kv = fold_kv(&mut tw);
    let tw2 = reboot(kv);
    // Balances re-derived from the journal alone.
    assert_eq!(bal(&tw2, 0), -500);
    assert_eq!(bal(&tw2, 1), 500);
    assert!(!store(&tw2).is_frozen(), "clean reboot is not frozen");
}

#[test]
fn boot_heals_a_lagging_projection_without_freezing() {
    // Mint (seq 0) then transfer (seq 1): player A ends at 700, last_journal_seq 1.
    let mut tw = boot();
    setup_mint(&mut tw, 1000);
    let q = tw.spawn_player("Quinn");
    tw.run(GOD, &format!("@account/open #{}=gold", q.0));
    tw.run(GOD, "@econ/pay 1=2 300");
    let mut kv = fold_kv(&mut tw);
    // LAG account 1's projection: it correctly reflects the mint (balance 1000 at
    // seq 0) but never saw the transfer (a dropped companion write).
    for (k, v) in kv.iter_mut() {
        if k == "account:1" {
            v["posted_balance"] = serde_json::json!(1000);
            v["last_journal_seq"] = serde_json::json!(0);
        }
    }
    let tw2 = reboot(kv);
    assert!(!store(&tw2).is_frozen(), "a benign lag must NOT freeze");
    assert_eq!(bal(&tw2, 1), 700, "the lagging projection is re-derived to the journal head");
}

#[test]
fn boot_freezes_on_a_drifted_projection() {
    let mut tw = boot();
    setup_mint(&mut tw, 1000);
    let q = tw.spawn_player("Quinn");
    tw.run(GOD, &format!("@account/open #{}=gold", q.0));
    tw.run(GOD, "@econ/pay 1=2 300"); // account 1 -> 700 at seq 1
    let mut kv = fold_kv(&mut tw);
    // DRIFT: claim to have seen seq 1 but record a balance the journal does not
    // support at that seq (700 is correct; 999 is a drifted projection).
    for (k, v) in kv.iter_mut() {
        if k == "account:1" {
            v["posted_balance"] = serde_json::json!(999);
            v["last_journal_seq"] = serde_json::json!(1);
        }
    }
    let tw2 = reboot(kv);
    assert!(store(&tw2).is_frozen(), "real drift must freeze the economy");
}

#[test]
fn boot_freezes_on_a_conservation_break() {
    // A hand-built journal whose single entry does NOT sum to zero (corruption the
    // pure engine can never produce) — the conservation tripwire must freeze.
    let mut tw = boot();
    let instrument = serde_json::json!({
        "id": "gold", "ruleset": "t", "code": "gold", "kind": "Currency", "scale": 0,
        "transferable": true, "negative_policy": "SystemOnly", "status": "Active", "revision": 1
    });
    let acct = |id: u64, bal: i64| serde_json::json!({
        "id": id, "owner": { "Object": 1 }, "instrument": "gold", "scope": null,
        "name": format!("a{id}"), "status": "Active", "posted_balance": bal, "held_amount": 0,
        "credit": { "allow_negative": true, "floor": null }, "last_journal_seq": 0, "revision": 1
    });
    // Two credit lines, +500 each — sum +1000, not zero.
    let zero_hash = vec![0u8; 32];
    let tx = serde_json::json!({
        "id": 0, "journal_seq": 0, "idempotency": { "namespace": "x", "key": "0" },
        "cause": { "Staff": { "reason": "corrupt" } }, "actor": { "Object": 1 }, "posted_at": 0,
        "description": "", "lines": [
            { "account": 1, "instrument": "gold", "delta": 500, "memo": "" },
            { "account": 2, "instrument": "gold", "delta": 500, "memo": "" }
        ], "reverses": null, "visibility": "Parties", "plan_hash": zero_hash
    });
    let chunk = LedgerChunk { chunk: 0, sealed: false, entries: vec![LedgerEntry { seq: 0, value: tx }] };
    let kv = vec![
        (SmolStr::new("instrument:gold"), instrument),
        (SmolStr::new("account:1"), acct(1, 500)),
        (SmolStr::new("account:2"), acct(2, 500)),
        (SmolStr::new("log:journal:0"), serde_json::to_value(&chunk).unwrap()),
    ];
    let res = try_boot(&mut tw, kv);
    assert!(res.is_ok(), "boot proceeds (frozen), does not error: {res:?}");
    assert!(store(&tw).is_frozen(), "a non-zero per-instrument sum freezes the economy");
}

#[test]
fn a_frozen_economy_refuses_posts() {
    // Boot into a frozen economy (conservation break), then a mint is refused.
    let mut tw = boot();
    setup_mint(&mut tw, 100); // acct 0 = -100, acct 1 = 100
    let mut kv = fold_kv(&mut tw);
    // Corrupt account 1's balance so the sum != 0 AND it drifts -> frozen.
    for (k, v) in kv.iter_mut() {
        if k == "account:1" {
            v["posted_balance"] = serde_json::json!(999);
        }
    }
    let mut tw2 = reboot(kv);
    assert!(store(&tw2).is_frozen());
    // A mint into the frozen economy is refused.
    assert!(has_error(&tw2.run(GOD, "@econ/mint 0=1 50")), "frozen economy refuses posting");
}

#[test]
fn an_idempotency_hit_still_requests_the_durable_gate() {
    // R-R2 regression: an AlreadyPosted HIT must not confirm ahead of the original's
    // durability — it still rides the strict durable gate.
    let mut tw = boot();
    setup_mint(&mut tw, 500); // transaction 0
    // Reverse tx 0 (posts the reversal), then drain the gate that post requested.
    assert!(!has_error(&tw.run(GOD, "@econ/reverse 0")), "first reverse");
    let _ = tw.engine_mut().take_durable_gate();
    // Reverse tx 0 AGAIN → AlreadyPosted (deterministic reversal key) → must gate.
    assert!(!has_error(&tw.run(GOD, "@econ/reverse 0")), "second reverse is an idempotent HIT");
    assert!(
        matches!(tw.engine_mut().take_durable_gate(), Some((_, true))),
        "an AlreadyPosted HIT still rides the strict durable gate (R-R2)"
    );
}

#[test]
fn mint_and_reverse_are_staff_only() {
    let mut tw = boot();
    setup_mint(&mut tw, 1000);
    let joe = tw.spawn_player("Joe");
    assert!(has_error(&tw.run(joe, "@econ/mint 0=1 100")), "mint refused for non-staff");
    assert!(has_error(&tw.run(joe, "@econ/reverse 0")), "reverse refused for non-staff");
    assert!(has_error(&tw.run(joe, "@instrument/register silver=0")), "register refused for non-staff");
}

#[test]
fn destroying_an_owner_freezes_their_account() {
    let mut tw = boot();
    let p = setup_mint(&mut tw, 1000);
    tw.take_module_ops();
    tw.world_mut().destroy(p).unwrap();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().emit_world_event(&WorldEvent::ObjDestroyed { obj: p }, &mut out, &mut fx);
    // The account is FROZEN (money preserved, no new posting), not deleted.
    assert_eq!(store(&tw).snapshot().account(AccountId(1)).unwrap().status, mush_economy::AccountStatus::Frozen);
    assert_eq!(bal(&tw, 1), 1000, "balance preserved");
    assert!(tw.take_module_ops().iter().any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == "account:1")), "frozen account re-persisted");
}

#[test]
fn boot_freezes_on_a_projection_ahead_of_the_journal() {
    // The lost-tail direction of the E2 pair (back-filled in E3b1 alongside the asset
    // quartet): a projection that remembers a journal entry the journal no longer has
    // is not a lag to heal — the tail it was written from is GONE.
    let mut tw = boot();
    setup_mint(&mut tw, 1000); // account 1 = 1000 at seq 0
    let mut kv = fold_kv(&mut tw);
    for (k, v) in kv.iter_mut() {
        if k == "account:1" {
            v["last_journal_seq"] = serde_json::json!(7);
        }
    }
    let tw2 = reboot(kv);
    assert!(store(&tw2).is_frozen(), "a projection ahead of the journal (lost tail) must freeze");
}

#[test]
fn boot_refuses_a_missing_account_doc() {
    // The account metadata was lost while its money history survived: the journal
    // cannot be replayed onto an account nothing describes, so boot FAILS rather than
    // inventing one (the safe failure is no spending authority at all).
    let mut tw = boot();
    setup_mint(&mut tw, 1000);
    let mut kv = fold_kv(&mut tw);
    kv.retain(|(k, _)| k != "account:1");
    let res = try_boot(&mut tw, kv);
    assert!(res.is_err(), "a journal line referencing an account with no doc must refuse the boot");
    assert!(
        res.unwrap_err().contains("unknown account"),
        "the refusal should name the cause"
    );
}
