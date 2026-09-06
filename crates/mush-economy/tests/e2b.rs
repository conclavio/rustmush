//! E2b acceptance: the EconomyModule attaches the pure E2a engine to a real world —
//! staff instrument/account setup, minting + transfer + reversal through the
//! validated post seam, per-record persistence, and the money-safety boot: full
//! journal re-derivation, directional self-heal of a lagging projection, and a
//! freeze on drift / a conservation break. Driven through the real Engine via
//! `TestWorld`.

use std::collections::HashMap;

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, Effect, Effects, GOD, GameModule, LedgerChunk,
    LedgerEntry, ModuleBootData, ModuleWrite, ObjId, Outbox, StoreReadResult, WorldCommand,
    WorldEvent,
};
use mush_economy::{
    AccountId, EconomicCause, EconomyConfig, EconomyPlan, EconomyStore, IdempotencyKey,
    PostOutcome, RequestedLine, TransactionId,
};
use mush_org::PrincipalId;
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
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn bal(tw: &TestWorld, id: u64) -> i64 {
    store(tw)
        .snapshot()
        .account(AccountId(id))
        .expect("account")
        .posted_balance
        .minor()
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
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    tw
}
fn try_boot(tw: &mut TestWorld, kv: Vec<(SmolStr, serde_json::Value)>) -> Result<(), String> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .modules_loaded(
            HashMap::from([(
                SmolStr::new(NS),
                ModuleBootData {
                    kv,
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )]),
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
    assert!(
        !has_error(&tw.run(GOD, "@account/system faucet=gold")),
        "system account"
    );
    assert!(
        !has_error(&tw.run(GOD, &format!("@account/open #{}=gold", p.0))),
        "player account"
    );
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
    let sum: i128 = store(&tw)
        .snapshot()
        .accounts()
        .map(|a| a.posted_balance.widen())
        .sum();
    assert_eq!(sum, 0);
}

#[test]
fn a_post_persists_the_bundle_and_survives_reboot() {
    let mut tw = boot();
    setup_mint(&mut tw, 500);
    // The mint persisted the journal chunk + both account docs + meta.
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(|o| o.ns == NS && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("log:journal:"))), "journal chunk persisted");
    assert!(
        ops.iter().any(|o| o.ns == NS
            && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == "account:1")),
        "account persisted"
    );
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
    assert_eq!(
        bal(&tw2, 1),
        700,
        "the lagging projection is re-derived to the journal head"
    );
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
    assert!(
        store(&tw2).is_frozen(),
        "real drift must freeze the economy"
    );
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
    let acct = |id: u64, bal: i64| {
        serde_json::json!({
            "id": id, "owner": { "Object": 1 }, "instrument": "gold", "scope": null,
            "name": format!("a{id}"), "status": "Active", "posted_balance": bal, "held_amount": 0,
            "credit": { "allow_negative": true, "floor": null }, "last_journal_seq": 0, "revision": 1
        })
    };
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
    let chunk = LedgerChunk {
        chunk: 0,
        sealed: false,
        entries: vec![LedgerEntry { seq: 0, value: tx }],
    };
    let kv = vec![
        (SmolStr::new("instrument:gold"), instrument),
        (SmolStr::new("account:1"), acct(1, 500)),
        (SmolStr::new("account:2"), acct(2, 500)),
        (
            SmolStr::new("log:journal:0"),
            serde_json::to_value(&chunk).unwrap(),
        ),
    ];
    let res = try_boot(&mut tw, kv);
    assert!(
        res.is_ok(),
        "boot proceeds (frozen), does not error: {res:?}"
    );
    assert!(
        store(&tw).is_frozen(),
        "a non-zero per-instrument sum freezes the economy"
    );
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
    assert!(
        has_error(&tw2.run(GOD, "@econ/mint 0=1 50")),
        "frozen economy refuses posting"
    );
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
    assert!(
        !has_error(&tw.run(GOD, "@econ/reverse 0")),
        "second reverse is an idempotent HIT"
    );
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
    assert!(
        has_error(&tw.run(joe, "@econ/mint 0=1 100")),
        "mint refused for non-staff"
    );
    assert!(
        has_error(&tw.run(joe, "@econ/reverse 0")),
        "reverse refused for non-staff"
    );
    assert!(
        has_error(&tw.run(joe, "@instrument/register silver=0")),
        "register refused for non-staff"
    );
}

#[test]
fn destroying_an_owner_freezes_their_account() {
    let mut tw = boot();
    let p = setup_mint(&mut tw, 1000);
    tw.take_module_ops();
    tw.world_mut().destroy(p).unwrap();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: p }, &mut out, &mut fx);
    // The account is FROZEN (money preserved, no new posting), not deleted.
    assert_eq!(
        store(&tw).snapshot().account(AccountId(1)).unwrap().status,
        mush_economy::AccountStatus::Frozen
    );
    assert_eq!(bal(&tw, 1), 1000, "balance preserved");
    assert!(
        tw.take_module_ops()
            .iter()
            .any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == "account:1")),
        "frozen account re-persisted"
    );
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
    assert!(
        store(&tw2).is_frozen(),
        "a projection ahead of the journal (lost tail) must freeze"
    );
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
    assert!(
        res.is_err(),
        "a journal line referencing an account with no doc must refuse the boot"
    );
    assert!(
        res.unwrap_err().contains("unknown account"),
        "the refusal should name the cause"
    );
}

// ----------------------------------------------------------------------
// H4 (E2-D24): the dispatch serial
// ----------------------------------------------------------------------

/// A command-caused post carries the world's per-boot dispatch correlation as
/// its serial (D-H-10), so two commands in one boot never share one. Pre-H4
/// entries carry `0`; nothing about them changes.
#[test]
fn h4_e2d24_two_commands_in_one_boot_carry_distinct_serials() {
    let mut tw = boot();
    let _p = setup_mint(&mut tw, 1000);
    let q = tw.spawn_player("Quinn");
    assert!(!has_error(
        &tw.run(GOD, &format!("@account/open #{}=gold", q.0))
    ));
    assert!(!has_error(&tw.run(GOD, "@econ/pay 1=2 10")), "first pay");
    assert!(!has_error(&tw.run(GOD, "@econ/pay 1=2 10")), "second pay");
    let serials: Vec<u64> = store(&tw)
        .transactions()
        .filter_map(|t| match t.cause {
            mush_economy::EconomicCause::Command { serial } => Some(serial),
            _ => None,
        })
        .collect();
    assert_eq!(serials.len(), 2, "two command-caused posts: {serials:?}");
    assert_ne!(serials[0], serials[1], "one boot, one serial each");
}

/// J1 (doc 12 §7): the journal arriving as text — the production loader's
/// form — boots to exactly the state it boots to from values (every pre-J1
/// rig's form, still moved to text by the engine), and the replay stats say
/// what they replayed.
#[test]
fn j1_boot_from_ledger_text_equals_boot_from_values() {
    let mut tw = boot();
    setup_mint(&mut tw, 500);
    let kv = fold_kv(&mut tw);
    assert!(kv.iter().any(|(k, _)| k.starts_with("log:journal:")));

    let from_values = reboot(kv.clone());
    let mut data = ModuleBootData {
        kv,
        ..Default::default()
    };
    data.normalize();
    assert!(!data.ledger.is_empty());
    assert!(data.kv.iter().all(|(k, _)| !k.starts_with("log:")));
    let mut from_text = boot();
    from_text.modules_loaded(HashMap::from([(SmolStr::new(NS), data)]));

    assert_eq!(bal(&from_values, 0), bal(&from_text, 0));
    assert_eq!(bal(&from_values, 1), bal(&from_text, 1));
    assert_eq!(bal(&from_text, 1), 500);
    let (a, b) = (store(&from_values), store(&from_text));
    assert_eq!(a.transactions().count(), b.transactions().count());
    assert_eq!(a.journal_len(), b.journal_len());
    let (ra, rb) = (a.boot_replay().unwrap(), b.boot_replay().unwrap());
    assert_eq!((ra.entries, ra.fetch_ms), (rb.entries, rb.fetch_ms));
    assert_eq!(ra.entries, 1);
    assert_eq!(b.posts_since_boot(), 0, "a replay is not a post");
}

// ----------------------------------------------------------------------
// J2: the bounded RAM tail (doc 12 §5.1 / §7 J2)
// ----------------------------------------------------------------------

/// The e2b rig with the RAM tail capped at `max` bodies.
fn boot_with_tail(max: usize) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new().with_config(EconomyConfig {
                tx_tail_max: max,
                ..EconomyConfig::default()
            }),
        ),
    ])
}

fn reboot_with_tail(kv: Vec<(SmolStr, serde_json::Value)>, max: usize) -> TestWorld {
    let mut tw = boot_with_tail(max);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    tw
}

/// One sweep fire at wall-clock second `t` (the e4b1 helper).
fn sweep_at(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

fn mint_n(tw: &mut TestWorld, n: usize) {
    for i in 0..n {
        let ev = tw.run(GOD, "@econ/mint 0=1 1");
        assert!(!has_error(&ev), "mint {i}: {}", texts(&ev));
    }
}

/// The one `StoreRead` effect a command deferred, as `(correlation, keys)`.
fn one_read(effects: &[Effect]) -> (u64, Vec<SmolStr>) {
    match effects {
        [
            Effect::StoreRead {
                module,
                correlation,
                keys,
                ..
            },
        ] => {
            assert_eq!(module, NS);
            (*correlation, keys.clone())
        }
        other => panic!("expected exactly one StoreRead effect: {other:?}"),
    }
}

/// Serve a read from the persisted documents the way the world task's driver
/// does: one document per key, absent keys reported.
fn serve(kv: &[(SmolStr, serde_json::Value)], keys: Vec<SmolStr>) -> StoreReadResult {
    let mut docs = Vec::new();
    let mut absent = Vec::new();
    for key in keys {
        match kv.iter().find(|(k, _)| *k == key) {
            Some((k, v)) => docs.push((k.clone(), v.clone())),
            None => absent.push(key),
        }
    }
    StoreReadResult::Docs { docs, absent }
}

fn complete(tw: &mut TestWorld, correlation: u64, result: StoreReadResult) -> String {
    let events = tw.apply(WorldCommand::StoreReadDone {
        module: SmolStr::new(NS),
        actor: GOD,
        correlation,
        payload: result.to_payload(),
    });
    texts(&events)
}

/// A post never evicts; the sweep evicts the tail down to its limit, oldest
/// first; the index keeps every id, its sequence and its chunk; and a reboot
/// under the same limit comes up with the same tail and the same index (lens B
/// and C).
#[test]
fn j2_the_tail_is_bounded_by_the_sweep_and_the_index_outlives_it() {
    let mut tw = boot_with_tail(3);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 5);
    {
        let s = store(&tw);
        assert_eq!(s.journal_len(), 6);
        assert_eq!(s.tail_len(), 6, "a post never evicts");
        assert_eq!(s.tx_tail_max(), 3);
    }
    sweep_at(&mut tw, 100);
    {
        let s = store(&tw);
        assert_eq!(s.tail_len(), 3, "the sweep evicts down to the limit");
        assert_eq!(s.tail_floor_seq(), Some(3));
        assert!(s.transaction(TransactionId(0)).is_none(), "the body left");
        let r = s
            .transaction_ref(TransactionId(0))
            .expect("the index keeps every id");
        assert_eq!(r.seq, 0);
        assert!(s.transaction(TransactionId(5)).is_some());
        assert!(
            s.transaction_ref(TransactionId(6)).is_none(),
            "never existed"
        );
        assert_eq!(s.chunk_of_seq(0), Some(0));
        assert_eq!(s.chunk_of_seq(5), Some(0));
    }
    assert_eq!(bal(&tw, 1), 6);
    let kv = fold_kv(&mut tw);
    let tw2 = reboot_with_tail(kv, 3);
    let s = store(&tw2);
    assert!(!s.is_frozen());
    assert_eq!(
        (s.journal_len(), s.tail_len(), s.tail_floor_seq()),
        (6, 3, Some(3))
    );
    assert_eq!(s.transaction_ref(TransactionId(0)).map(|r| r.seq), Some(0));
    assert_eq!(s.transaction_ref(TransactionId(5)).map(|r| r.seq), Some(5));
    assert_eq!(s.chunk_of_seq(2), Some(0));
    assert_eq!(bal(&tw2, 1), 6);
    // J2-F2: the replay stats count the journal, not the tail.
    assert_eq!(s.boot_replay().map(|r| r.entries), Some(6));
}

/// `@econ/history` renders the tail's rows at once, then reads the sealed
/// chunks below the tail off the world loop and renders the rest through the
/// same rows, newest first (lens P).
#[test]
fn j2_history_reaches_past_the_tail_through_the_journal() {
    let mut tw = boot_with_tail(2);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 39); // 40 entries: chunk 0 sealed at 16 KiB, chunk 1 open
    sweep_at(&mut tw, 100);
    assert_eq!(store(&tw).tail_len(), 2);
    let kv = fold_kv(&mut tw);
    assert!(
        kv.iter().any(|(k, _)| k == "log:journal:1"),
        "the journal spans two chunks: {:?}",
        kv.iter()
            .map(|(k, _)| k.as_str())
            .filter(|k| k.starts_with("log:"))
            .collect::<Vec<_>>()
    );

    let (events, effects) = tw.run_with_effects(GOD, "@econ/history 1 20");
    let text = texts(&events);
    assert!(
        text.contains("tx #39 at"),
        "the tail's rows come first: {text}"
    );
    assert!(text.contains("tx #38 at"), "{text}");
    assert!(
        !text.contains("tx #37 at"),
        "the tail holds two bodies: {text}"
    );
    assert!(text.contains("Reading older history"), "{text}");
    let (correlation, keys) = one_read(&effects);
    assert!(
        keys.iter().all(|k| k.starts_with("log:journal:")),
        "{keys:?}"
    );

    let text = complete(&mut tw, correlation, serve(&kv, keys));
    for id in (20..=37).rev() {
        assert!(
            text.contains(&format!("tx #{id} at")),
            "missing #{id}: {text}"
        );
    }
    assert!(
        !text.contains("tx #19 at"),
        "capped at the 20 asked for: {text}"
    );
    // J2-F1: the open chunk holds the tail's bodies too; the page renders only
    // what lies below the tail's floor, never the rows the command already sent.
    assert!(
        !text.contains("tx #39 at") && !text.contains("tx #38 at"),
        "the tail's rows were repeated: {text}"
    );
    // The pending read is spent: a replayed completion is a stale drop.
    let again = complete(&mut tw, correlation, serve(&kv, Vec::new()));
    assert!(
        again.is_empty(),
        "a completion nobody waits on renders nothing: {again}"
    );
}

/// `@econ/reverse` of a transaction whose body left the tail reads it from the
/// journal off the world loop and reverses it on re-entry; the reversal is the
/// engine's, and a replayed completion posts nothing twice (lens P).
#[test]
fn j2_reverse_reaches_past_the_tail_through_the_journal() {
    let mut tw = boot_with_tail(2);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 4);
    sweep_at(&mut tw, 100);
    assert!(store(&tw).transaction(TransactionId(0)).is_none());
    let kv = fold_kv(&mut tw);

    let (events, effects) = tw.run_with_effects(GOD, "@econ/reverse #0");
    let text = texts(&events);
    assert!(text.contains("older than the in-memory tail"), "{text}");
    assert_eq!(bal(&tw, 1), 5, "nothing moved yet");
    let (correlation, keys) = one_read(&effects);
    assert_eq!(keys, vec![SmolStr::new("log:journal:0")]);

    let text = complete(&mut tw, correlation, serve(&kv, keys.clone()));
    assert!(text.contains("Reversed: transaction #5 posted."), "{text}");
    assert_eq!(bal(&tw, 1), 4);
    {
        let s = store(&tw);
        assert_eq!(
            s.transaction(TransactionId(5)).map(|t| t.reverses),
            Some(Some(TransactionId(0)))
        );
        assert_eq!(s.transaction_ref(TransactionId(5)).map(|r| r.seq), Some(5));
    }
    let again = complete(&mut tw, correlation, serve(&kv, keys));
    assert!(again.is_empty(), "{again}");
    assert_eq!(bal(&tw, 1), 4, "a stale completion posts nothing");

    // A transaction the journal never had is refused at once, with no read.
    let (events, effects) = tw.run_with_effects(GOD, "@econ/reverse #77");
    assert!(
        texts(&events).contains("no such transaction"),
        "{}",
        texts(&events)
    );
    assert!(effects.is_empty());
}

// ----------------------------------------------------------------------
// J3: the checkpoint (doc 12 §5.2–§5.5, §7 J3)
// ----------------------------------------------------------------------

/// A test-only module with one command, `@testpost <key> <amount>`: a mint
/// from the faucet to account 1 under an idempotency key the test chooses —
/// the way to re-post a pre-checkpoint key after a checkpoint boot (lens I).
struct TestPostMod;

impl GameModule for TestPostMod {
    fn name(&self) -> &'static str {
        "e2btestpost"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        reg.command(
            CommandSpec {
                name: "@testpost".into(),
                shape: ArgShape::Raw,
                help: "@testpost <key> <amount>   test: mint under a chosen idempotency key".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let raw = ctx.args.raw().to_string();
                let mut words = raw.split_whitespace();
                let key = words.next().unwrap_or("k").to_string();
                let amount: i64 = words
                    .next()
                    .and_then(|w| w.parse().ok())
                    .ok_or_else(|| CmdError::msg("Usage: @testpost <key> <amount>"))?;
                let plan = EconomyPlan::new(
                    IdempotencyKey::new("t", key),
                    PrincipalId::Object(ctx.actor),
                    EconomicCause::Staff {
                        reason: "test".into(),
                    },
                    vec![
                        RequestedLine {
                            account: AccountId(0),
                            delta: mush_economy::Amount::from_minor(-amount),
                            memo: "mint".into(),
                        },
                        RequestedLine {
                            account: AccountId(1),
                            delta: mush_economy::Amount::from_minor(amount),
                            memo: "grant".into(),
                        },
                    ],
                );
                let outcome = mush_economy::post(ctx, &plan);
                let actor = ctx.actor;
                let line = match outcome {
                    Ok(PostOutcome::Posted(id)) => format!("posted #{id}"),
                    Ok(PostOutcome::AlreadyPosted(id)) => format!("already #{id}"),
                    Ok(other) => format!("other {other:?}"),
                    Err(e) => format!("refused {e}"),
                };
                ctx.out.sys_to_actor(actor, "test", line);
                Ok(())
            }),
        )?;
        Ok(())
    }
}

/// The e2b rig with the RAM tail and the checkpoint cadence set, plus the
/// test post command.
fn boot_j3(tail: usize, every: u64) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new().with_config(EconomyConfig {
                tx_tail_max: tail,
                checkpoint_every_entries: every,
                ..EconomyConfig::default()
            }),
        ),
        Box::new(TestPostMod),
    ])
}

fn reboot_j3(kv: Vec<(SmolStr, serde_json::Value)>, tail: usize, every: u64) -> TestWorld {
    let mut tw = boot_j3(tail, every);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    tw
}

/// Overlay the world's new module ops on `base` (last write wins, deletes
/// remove): what a store holds after those ops committed.
fn fold_into(
    mut base: Vec<(SmolStr, serde_json::Value)>,
    tw: &mut TestWorld,
) -> Vec<(SmolStr, serde_json::Value)> {
    for op in tw.take_module_ops() {
        if op.ns != NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                base.retain(|(k, _)| *k != key);
                base.push((key, value));
            }
            ModuleWrite::Del { key } => base.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    base
}

fn checkpoint_keys(kv: &[(SmolStr, serde_json::Value)]) -> Vec<String> {
    let mut keys: Vec<String> = kv
        .iter()
        .filter(|(k, _)| k.starts_with("checkpoint:"))
        .map(|(k, _)| k.to_string())
        .collect();
    keys.sort();
    keys
}

fn patch(kv: &mut [(SmolStr, serde_json::Value)], key: &str, f: impl Fn(&mut serde_json::Value)) {
    let mut seen = false;
    for (k, v) in kv.iter_mut() {
        if k == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key: {key}");
}

fn replay(tw: &TestWorld) -> (Option<u64>, usize, usize) {
    let r = store(tw).boot_replay().expect("booted");
    (r.from_checkpoint, r.replayed, r.entries)
}

/// J3-F3 (cadence-1 verify fold): sequence 0 is both the journal's first
/// sequence and the never-touched sentinel, and the boot's checkpoint rule
/// first excluded every claim of 0 — so an account whose only touch was the
/// journal's first entry read as drift after a checkpoint boot and froze. The
/// fold's last touch decides: a claim at the touch is judged, a touch past the
/// claim (the sentinel on an account the journal did touch) lags and heals, and
/// a claim the journal never made is still B5.
#[test]
fn j3_a_first_entry_touch_and_a_sentinel_lag_survive_a_checkpoint_boot() {
    let mut tw = boot_j3(20_000, 1);
    setup_mint(&mut tw, 1); // seq 0: the faucet and account 1, never again
    let q = tw.spawn_player("Quinn");
    assert!(!has_error(
        &tw.run(GOD, &format!("@account/open #{}=gold", q.0))
    ));
    let ev = tw.run(GOD, "@econ/mint 0=2 1"); // seq 1: the faucet and account 2
    assert!(!has_error(&ev), "second mint: {}", texts(&ev));
    sweep_at(&mut tw, 100);
    assert_eq!(store(&tw).checkpoint().map(|c| c.covered_seq), Some(1));
    let kv = fold_into(Vec::new(), &mut tw);

    let tw2 = reboot_j3(kv.clone(), 20_000, 1);
    assert_eq!(replay(&tw2), (Some(1), 0, 2));
    assert!(
        !store(&tw2).is_frozen(),
        "an account whose only touch is seq 0 is judged at its value, not read as the sentinel"
    );
    assert_eq!((bal(&tw2, 1), bal(&tw2, 2)), (1, 1));

    // The sentinel on an account the journal touched: the doc write of the
    // second mint lost, so account 2 still reads as never touched. It lags.
    let mut lost = kv.clone();
    patch(&mut lost, "account:2", |v| {
        v["posted_balance"] = serde_json::json!(0);
        v["last_journal_seq"] = serde_json::json!(0);
    });
    let tw3 = reboot_j3(lost, 20_000, 1);
    assert!(
        !store(&tw3).is_frozen(),
        "a sentinel the journal outran lags and heals"
    );
    assert_eq!(bal(&tw3, 2), 1);

    // B5 under a checkpoint: a claim the journal never made still freezes.
    let mut impossible = kv;
    patch(&mut impossible, "account:1", |v| {
        v["last_journal_seq"] = serde_json::json!(1);
    });
    let tw4 = reboot_j3(impossible, 20_000, 1);
    assert!(
        store(&tw4).is_frozen(),
        "a doc claiming a sequence it never appears at is B5, checkpoint or not"
    );
}

/// J3-F4 (cadence-1 verify fold): a checkpointed boot used to decode nothing
/// below the covered chunk, so the RAM tail came up empty and every history
/// page and reversal went cold until new posts refilled it. The chunks holding
/// the tail are decoded whatever the checkpoint covers; their entries are kept,
/// not folded.
#[test]
fn j3_the_tail_is_kept_across_a_checkpoint_boot() {
    let mut tw = boot_j3(3, 1);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 5); // seq 0..=5
    sweep_at(&mut tw, 100);
    assert_eq!(store(&tw).checkpoint().map(|c| c.covered_seq), Some(5));
    assert_eq!(store(&tw).tail_len(), 3, "the sweep evicted to the limit");
    let kv = fold_into(Vec::new(), &mut tw);

    let tw2 = reboot_j3(kv, 3, 1);
    assert_eq!(replay(&tw2), (Some(5), 0, 6));
    let s = store(&tw2);
    assert!(!s.is_frozen());
    assert_eq!(
        (s.journal_len(), s.tail_len(), s.tail_floor_seq()),
        (6, 3, Some(3)),
        "the newest bodies are in RAM after a checkpoint boot"
    );
    assert!(s.transaction(TransactionId(5)).is_some());
    assert!(
        s.transaction(TransactionId(2)).is_none(),
        "below the tail: index only"
    );
    assert_eq!(s.transaction_ref(TransactionId(0)).map(|r| r.seq), Some(0));
    assert_eq!(bal(&tw2, 1), 6);
}

/// The sweep writes a checkpoint at its cadence; a boot seeds from it and
/// replays only the tail; a second checkpoint inherits the first's part and
/// sweeps the head it supersedes' predecessor.
#[test]
fn j3_the_sweep_writes_a_checkpoint_and_boot_resumes_from_it() {
    let mut tw = boot_j3(20_000, 4);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 5); // seq 0..=5
    assert!(store(&tw).checkpoint().is_none());
    sweep_at(&mut tw, 100);
    let cp = store(&tw)
        .checkpoint()
        .cloned()
        .expect("written at the cadence");
    assert_eq!((cp.covered_seq, cp.parts.len(), cp.max_txid), (5, 1, 5));
    let kv = fold_into(Vec::new(), &mut tw);
    assert_eq!(
        checkpoint_keys(&kv),
        vec![
            "checkpoint:5".to_string(),
            "checkpoint:5:idem:0".to_string()
        ]
    );

    let tw2 = reboot_j3(kv.clone(), 20_000, 4);
    assert_eq!(replay(&tw2), (Some(5), 0, 6));
    assert_eq!(bal(&tw2, 1), 6);
    assert!(!store(&tw2).is_frozen());
    assert_eq!(
        store(&tw2).transaction_ref(TransactionId(0)).map(|r| r.seq),
        Some(0)
    );
    assert_eq!(store(&tw2).checkpoint().map(|c| c.covered_seq), Some(5));
    assert_eq!(store(&tw2).created_count(ObjId(1)), 0);

    // The tail past the checkpoint replays; a later checkpoint inherits the
    // part and sweeps nothing yet (two heads are kept).
    let mut tw2 = tw2;
    mint_n(&mut tw2, 3); // seq 6..=8: below the cadence, so no checkpoint yet
    let kv2 = fold_into(kv, &mut tw2);
    let tw3 = reboot_j3(kv2.clone(), 20_000, 4);
    assert_eq!(replay(&tw3), (Some(5), 3, 9));
    assert_eq!(bal(&tw3, 1), 9);
    let mut tw3 = tw3;
    sweep_at(&mut tw3, 1_000);
    assert_eq!(
        store(&tw3).checkpoint().map(|c| c.covered_seq),
        Some(5),
        "three new entries are under the cadence of four"
    );
    mint_n(&mut tw3, 1); // seq 9: the fourth
    sweep_at(&mut tw3, 2_000);
    let cp = store(&tw3)
        .checkpoint()
        .cloned()
        .expect("the second checkpoint");
    assert_eq!((cp.covered_seq, cp.parts.len()), (9, 2));
    let kv3 = fold_into(kv2, &mut tw3);
    assert_eq!(
        checkpoint_keys(&kv3),
        vec![
            "checkpoint:5".to_string(),
            "checkpoint:5:idem:0".to_string(),
            "checkpoint:9".to_string(),
            "checkpoint:9:idem:1".to_string(),
        ]
    );
    // A third sweeps the first head; the part it introduced is still inherited.
    mint_n(&mut tw3, 4); // seq 10..=13
    sweep_at(&mut tw3, 3_000);
    let kv4 = fold_into(kv3, &mut tw3);
    assert_eq!(
        checkpoint_keys(&kv4),
        vec![
            "checkpoint:13".to_string(),
            "checkpoint:13:idem:2".to_string(),
            "checkpoint:5:idem:0".to_string(),
            "checkpoint:9".to_string(),
            "checkpoint:9:idem:1".to_string(),
        ]
    );
    let tw4 = reboot_j3(kv4, 20_000, 4);
    assert_eq!(replay(&tw4), (Some(13), 0, 14));
    assert_eq!(bal(&tw4, 1), 14);
}

/// Lens V: every way a checkpoint can be wrong is ignored — a missing part, a
/// tampered part, a wrong state hash, a head that does not cover what its key
/// says — and the boot replays the whole journal, unfrozen, with the right
/// balances. Lens T: parts with no head, or a head with no parts, the same.
#[test]
fn j3_an_invalid_checkpoint_is_ignored_and_the_journal_replayed_whole() {
    let mut tw = boot_j3(20_000, 2);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 3);
    sweep_at(&mut tw, 100);
    let kv = fold_into(Vec::new(), &mut tw);
    assert_eq!(checkpoint_keys(&kv).len(), 2);
    let full = (None, 4, 4);

    let mut missing_part = kv.clone();
    missing_part.retain(|(k, _)| k != "checkpoint:3:idem:0");
    let tw2 = reboot_j3(missing_part, 20_000, 2);
    assert_eq!(replay(&tw2), full, "a missing part");
    assert_eq!(bal(&tw2, 1), 4);
    assert!(!store(&tw2).is_frozen());

    let mut tampered = kv.clone();
    patch(&mut tampered, "checkpoint:3:idem:0", |v| {
        v["rows"][0]["posted_at"] = serde_json::json!(999);
    });
    let tw3 = reboot_j3(tampered, 20_000, 2);
    assert_eq!(replay(&tw3), full, "a tampered part");
    assert_eq!(bal(&tw3, 1), 4);

    let mut bad_hash = kv.clone();
    patch(&mut bad_hash, "checkpoint:3", |v| {
        v["state_hash"] = serde_json::json!("00".repeat(32));
    });
    let tw4 = reboot_j3(bad_hash, 20_000, 2);
    assert_eq!(replay(&tw4), full, "a wrong state hash");

    let mut wrong_seq = kv.clone();
    patch(&mut wrong_seq, "checkpoint:3", |v| {
        v["covered_seq"] = serde_json::json!(99);
    });
    let tw5 = reboot_j3(wrong_seq, 20_000, 2);
    assert_eq!(replay(&tw5), full, "a head not covering what its key says");

    let mut headless = kv.clone();
    headless.retain(|(k, _)| k != "checkpoint:3");
    let tw6 = reboot_j3(headless, 20_000, 2);
    assert_eq!(replay(&tw6), full, "parts with no head");

    // A garbage newer head does not shadow the valid older one.
    let mut garbage = kv.clone();
    garbage.push((
        SmolStr::new("checkpoint:7"),
        serde_json::json!({ "format": 1, "nope": true }),
    ));
    let tw7 = reboot_j3(garbage, 20_000, 2);
    assert_eq!(replay(&tw7), (Some(3), 0, 4), "the valid one is still used");
}

/// Lens D: drift at the head is still caught after a checkpoint boot; a
/// document lagging a pre-checkpoint version heals without a verdict.
#[test]
fn j3_drift_still_freezes_and_a_pre_checkpoint_lag_heals() {
    let mut tw = boot_j3(20_000, 2);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 3); // account 1: balance 4 at seq 3
    sweep_at(&mut tw, 100);
    let kv = fold_into(Vec::new(), &mut tw);

    let mut drifted = kv.clone();
    patch(&mut drifted, "account:1", |v| {
        v["posted_balance"] = serde_json::json!(40);
    });
    let tw2 = reboot_j3(drifted, 20_000, 2);
    assert_eq!(replay(&tw2).0, Some(3));
    assert!(
        store(&tw2).is_frozen(),
        "drift at the claimed head sequence freezes"
    );

    let mut lagging = kv.clone();
    patch(&mut lagging, "account:1", |v| {
        v["posted_balance"] = serde_json::json!(2);
        v["last_journal_seq"] = serde_json::json!(1);
    });
    let tw3 = reboot_j3(lagging, 20_000, 2);
    assert_eq!(replay(&tw3).0, Some(3));
    assert!(!store(&tw3).is_frozen(), "a pre-checkpoint lag heals");
    assert_eq!(bal(&tw3, 1), 4);
}

/// Lens I: a key posted before the checkpoint is still refused as already
/// posted after a checkpoint boot; a fresh key posts.
#[test]
fn j3_idempotency_survives_a_checkpoint_boot() {
    let mut tw = boot_j3(20_000, 2);
    setup_mint(&mut tw, 1);
    let first = texts(&tw.run(GOD, "@testpost alpha 5"));
    assert!(first.contains("posted #1"), "{first}");
    mint_n(&mut tw, 2);
    sweep_at(&mut tw, 100);
    let kv = fold_into(Vec::new(), &mut tw);
    let mut tw2 = reboot_j3(kv, 20_000, 2);
    assert_eq!(replay(&tw2).0, Some(3));
    let again = texts(&tw2.run(GOD, "@testpost alpha 5"));
    assert!(again.contains("already #1"), "{again}");
    assert_eq!(bal(&tw2, 1), 8);
    let fresh = texts(&tw2.run(GOD, "@testpost beta 1"));
    assert!(fresh.contains("posted #4"), "{fresh}");
    assert_eq!(bal(&tw2, 1), 9);
}

/// Lens F: a frozen economy writes no checkpoint; disabled cadence writes none.
#[test]
fn j3_no_checkpoint_while_frozen_or_disabled() {
    let mut tw = boot_j3(20_000, 0);
    setup_mint(&mut tw, 1);
    mint_n(&mut tw, 5);
    sweep_at(&mut tw, 100);
    assert!(store(&tw).checkpoint().is_none(), "disabled");
    let kv = fold_into(Vec::new(), &mut tw);
    let mut frozen = kv.clone();
    patch(&mut frozen, "account:1", |v| {
        v["posted_balance"] = serde_json::json!(40);
    });
    let mut tw2 = reboot_j3(frozen, 20_000, 2);
    assert!(store(&tw2).is_frozen());
    sweep_at(&mut tw2, 200);
    assert!(
        store(&tw2).checkpoint().is_none(),
        "frozen: nothing written"
    );
}
