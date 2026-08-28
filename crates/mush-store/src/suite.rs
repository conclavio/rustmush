//! Shared behavior suite: every [`ObjectStore`] implementation must pass
//! these, so the in-memory fake proves the exact semantics the Redis store
//! ships with.
//!
//! Each function takes an **empty** store and panics (via `assert!`) on any
//! contract violation. The unit tests run them against [`crate::InMemoryStore`]
//! unconditionally; `tests/redis_suite.rs` runs the same functions against a
//! live Redis when `REDIS_URL` is set.
//!
//! This module is `#[doc(hidden)] pub` only so integration tests can reach
//! it; it is not API.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mush_core::{
    Attr, AttrFlags, AttrValue, Flags, GOD, LIMBO, LockExpr, LockType, Obj, ObjId, ObjKind, Powers,
    World,
};

use crate::migrate::{Migration, run_migrations};
use crate::store::ObjectStore;
use crate::types::{AccountRec, ModuleOp, ModuleWrite, StoreError, WriteBatch};

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Snapshot a fresh minimal world (Limbo `#0` + God `#1`) into a seed batch,
/// returning the objects for later comparison.
fn minimal_seed_batch(serial: u64) -> (WriteBatch, Vec<Obj>) {
    let mut world = World::new_minimal();
    let dirty = world.take_dirty();
    let upserts: Vec<Obj> = dirty
        .iter()
        .filter_map(|id| world.get(*id).cloned())
        .collect();
    assert_eq!(upserts.len(), 2, "minimal world dirties exactly #0 and #1");
    let batch = WriteBatch {
        serial,
        upserts: upserts.clone(),
        next_id: Some(world.next_id()),
        durable: true,
        ..WriteBatch::default()
    };
    (batch, upserts)
}

fn thing(id: u32, name: &str) -> Obj {
    let mut o = Obj::new(ObjId(id), ObjKind::Thing, name, GOD, 100);
    o.location = Some(LIMBO);
    o.home = Some(LIMBO);
    o
}

/// Empty store reports `None`; a seeded minimal world loads back exactly and
/// hydrates through [`World::from_objects`].
pub async fn empty_seed_load_round_trip(store: &dyn ObjectStore) {
    assert!(
        store.load_world().await.expect("load empty").is_none(),
        "empty store must load as None"
    );

    let (batch, seeded) = minimal_seed_batch(1);
    let receipt = store.write(batch).await.expect("seed write");
    assert_eq!(receipt.serial, 1);
    store.set_schema_version(1).await.expect("stamp version");

    let snap = store
        .load_world()
        .await
        .expect("load seeded")
        .expect("seeded store must load as Some");
    assert_eq!(snap.next_id, 2);
    assert_eq!(snap.schema_version, 1);
    let mut objects = snap.objects.clone();
    objects.sort_by_key(|o| o.id);
    assert_eq!(objects, seeded, "objects must round-trip exactly");

    let world = World::from_objects(snap.objects, snap.next_id).expect("hydrate");
    assert_eq!(world.next_id(), 2);
    assert_eq!(world.contents_of(LIMBO), [GOD]);
}

/// One batch carrying deletes + upserts applies as a unit: upserts first,
/// deletes second (so an id in both lists ends up deleted), `next_id` last.
pub async fn batch_atomicity(store: &dyn ObjectStore) {
    let (seed, _) = minimal_seed_batch(1);
    store.write(seed).await.expect("seed");

    store
        .write(WriteBatch {
            serial: 2,
            upserts: vec![thing(2, "alpha"), thing(3, "beta")],
            next_id: Some(4),
            ..WriteBatch::default()
        })
        .await
        .expect("create alpha/beta");

    // One batch: delete alpha, rename beta, create delta, create-and-delete
    // ephemeral (#5) — the delete must win over the upsert in the same batch.
    store
        .write(WriteBatch {
            serial: 3,
            upserts: vec![thing(3, "gamma"), thing(4, "delta"), thing(5, "ephemeral")],
            deletes: vec![ObjId(2), ObjId(5)],
            next_id: Some(6),
            ..WriteBatch::default()
        })
        .await
        .expect("mixed batch");

    let snap = store.load_world().await.expect("load").expect("non-empty");
    assert_eq!(snap.next_id, 6);
    let mut ids: Vec<u32> = snap.objects.iter().map(|o| o.id.0).collect();
    ids.sort_unstable();
    assert_eq!(ids, [0, 1, 3, 4], "delete beats same-batch upsert");
    let renamed = snap.objects.iter().find(|o| o.id == ObjId(3)).unwrap();
    assert_eq!(renamed.name, "gamma");
    // The surviving set still hydrates.
    World::from_objects(snap.objects, snap.next_id).expect("hydrate after mixed batch");
}

/// `next_id` persists on its own (an id-allocation-only batch) and empty
/// batches are acknowledged without changing anything.
pub async fn next_id_persistence(store: &dyn ObjectStore) {
    let (seed, _) = minimal_seed_batch(1);
    store.write(seed).await.expect("seed");

    let receipt = store
        .write(WriteBatch {
            serial: 2,
            next_id: Some(42),
            ..WriteBatch::default()
        })
        .await
        .expect("bump next_id");
    assert_eq!(receipt.serial, 2);

    // Empty batch: acked, no effect.
    store
        .write(WriteBatch {
            serial: 3,
            ..WriteBatch::default()
        })
        .await
        .expect("empty batch");

    let snap = store.load_world().await.expect("load").expect("non-empty");
    assert_eq!(snap.next_id, 42);
    assert_eq!(snap.objects.len(), 2);
}

/// Two concurrent creates of the same account: exactly one wins, and the
/// stored hash belongs to the winner.
pub async fn account_reserve_race(store: &dyn ObjectStore) {
    let rec = |hash: &str| AccountRec {
        name: "Nick".to_string(),
        argon2_hash: hash.to_string(),
        created: now_unix(),
        characters: vec![],
    };
    let (a, b) = futures::join!(
        store.create_account(rec("hash-a")),
        store.create_account(rec("hash-b"))
    );
    let winners = usize::from(a.is_ok()) + usize::from(b.is_ok());
    assert_eq!(winners, 1, "exactly one concurrent create must win");
    let a_won = a.is_ok();
    let loser = if a_won { b } else { a };
    assert!(
        matches!(loser, Err(StoreError::AccountExists)),
        "loser must see AccountExists, got {loser:?}"
    );

    let stored = store
        .get_account("NICK")
        .await
        .expect("get")
        .expect("account exists");
    let expected = if a_won { "hash-a" } else { "hash-b" };
    assert_eq!(stored.argon2_hash, expected, "winner's doc must be stored");
}

/// Reserve → case-insensitive duplicate refused → get → link characters →
/// list; unknown accounts behave.
pub async fn account_lifecycle(store: &dyn ObjectStore) {
    assert!(
        store.get_account("nobody").await.expect("get").is_none(),
        "unknown account must be None"
    );
    assert!(matches!(
        store.link_character("ghost", ObjId(9), "Boo").await,
        Err(StoreError::NoSuchAccount(_))
    ));

    let created = now_unix();
    store
        .create_account(AccountRec {
            name: "Nick".to_string(),
            argon2_hash: "$argon2id$fake".to_string(),
            created,
            characters: vec![],
        })
        .await
        .expect("create");
    assert!(
        matches!(
            store
                .create_account(AccountRec {
                    name: "NICK".to_string(),
                    argon2_hash: "other".to_string(),
                    created,
                    characters: vec![],
                })
                .await,
            Err(StoreError::AccountExists)
        ),
        "reserve must be case-insensitive"
    );

    store
        .link_character("nIcK", ObjId(7), "Nicky")
        .await
        .expect("link 1");
    store
        .link_character("nick", ObjId(8), "Alt")
        .await
        .expect("link 2");
    let rec = store
        .get_account("nick")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.name, "Nick", "display case preserved");
    assert_eq!(rec.argon2_hash, "$argon2id$fake");
    assert_eq!(rec.created, created);
    let chars: Vec<(u32, &str)> = rec
        .characters
        .iter()
        .map(|c| (c.id.0, c.name.as_str()))
        .collect();
    assert_eq!(chars, [(7, "Nicky"), (8, "Alt")]);

    store
        .create_account(AccountRec {
            name: "Zoe".to_string(),
            argon2_hash: "h".to_string(),
            created,
            characters: vec![],
        })
        .await
        .expect("second account");
    let mut names: Vec<String> = store
        .list_accounts()
        .await
        .expect("list")
        .into_iter()
        .map(|a| a.name)
        .collect();
    names.sort();
    assert_eq!(names, ["Nick", "Zoe"]);
}

/// Active, permanent, and already-expired bans; clear semantics.
pub async fn ban_expiry(store: &dyn ObjectStore) {
    let now = now_unix();
    store
        .set_ban("1.2.3.4", Some(now + 3600), "flooding")
        .await
        .expect("timed ban");
    store
        .set_ban("2001:db8::7", None, "permanent")
        .await
        .expect("permanent ban");
    store
        .set_ban("9.9.9.9", Some(now - 10), "already lapsed")
        .await
        .expect("lapsed ban");

    let mut bans = store.get_bans().await.expect("get_bans");
    bans.sort_by(|a, b| a.ip.cmp(&b.ip));
    let ips: Vec<&str> = bans.iter().map(|b| b.ip.as_str()).collect();
    assert_eq!(
        ips,
        ["1.2.3.4", "2001:db8::7"],
        "expired ban must not appear"
    );
    assert_eq!(bans[0].until_unix, Some(now + 3600));
    assert_eq!(bans[0].reason, "flooding");
    assert_eq!(bans[1].until_unix, None);

    assert!(store.clear_ban("2001:db8::7").await.expect("clear"));
    assert!(!store.clear_ban("2001:db8::7").await.expect("re-clear"));
    assert!(!store.clear_ban("9.9.9.9").await.expect("clear lapsed"));
    let bans = store.get_bans().await.expect("get_bans");
    assert_eq!(bans.len(), 1);
    assert_eq!(bans[0].ip, "1.2.3.4");
}

/// Fresh stores are stamped without running anything; version bumps run each
/// pending migration exactly once; re-runs are no-ops.
pub async fn migration_version_bump(store: &dyn ObjectStore) {
    assert_eq!(store.get_schema_version().await.expect("get"), None);

    // Fresh store + v1-only registry: stamped straight to 1, nothing runs.
    let v = run_migrations(store, &crate::migrate::core_migrations())
        .await
        .expect("initial");
    assert_eq!(v, 1);
    assert_eq!(store.get_schema_version().await.expect("get"), Some(1));

    // Register a v2 step and count its executions.
    let ran = Arc::new(AtomicU32::new(0));
    let registry = || {
        let ran = Arc::clone(&ran);
        vec![
            Migration {
                to_version: 1,
                name: "initial schema",
                run: Box::new(|_| Box::pin(async { Ok(()) })),
            },
            Migration {
                to_version: 2,
                name: "test bump",
                run: Box::new(move |_store| {
                    ran.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Ok(()) })
                }),
            },
        ]
    };
    let v = run_migrations(store, &registry()).await.expect("bump to 2");
    assert_eq!(v, 2);
    assert_eq!(store.get_schema_version().await.expect("get"), Some(2));
    assert_eq!(ran.load(Ordering::SeqCst), 1, "v2 ran exactly once");

    // Re-run: no-op.
    let v = run_migrations(store, &registry()).await.expect("re-run");
    assert_eq!(v, 2);
    assert_eq!(ran.load(Ordering::SeqCst), 1, "no re-execution");

    // A store newer than the binary refuses.
    assert!(matches!(
        run_migrations(store, &crate::migrate::core_migrations()).await,
        Err(StoreError::Migration(_))
    ));
}

/// A maximally populated [`Obj`] survives the store path bit-for-bit — the
/// serde stability check for the `obj:{id}` JSON doc format.
pub async fn serde_obj_round_trip(store: &dyn ObjectStore) {
    let mut o = Obj::new(
        ObjId(5),
        ObjKind::Player,
        "Zanzibar",
        ObjId(5),
        1_700_000_000,
    );
    o.aliases = vec!["Zan".into(), "Zz".into()];
    o.location = Some(LIMBO);
    o.destination = Some(LIMBO);
    o.home = Some(LIMBO);
    o.zone = Some(LIMBO);
    o.parent = Some(GOD);
    o.flags = Flags::WIZARD;
    o.powers = Powers::all();
    o.markers.insert("wod:kindred".into());
    o.locks.insert(LockType::Basic, LockExpr::Is(GOD));
    o.attrs.set("Desc", Attr::text("A tall tale.", GOD));
    o.attrs.set(
        "sheet:wod",
        Attr::json(
            serde_json::json!({"clan": "Nosferatu", "dots": [1, 2, 3]}),
            ObjId(5),
        ),
    );
    o.attrs.set(
        "secret",
        Attr {
            value: AttrValue::Text("hidden".into()),
            owner: GOD,
            flags: AttrFlags::HIDDEN | AttrFlags::WIZARD_ONLY,
        },
    );
    o.modified = 1_700_000_777;

    store
        .write(WriteBatch {
            serial: 1,
            upserts: vec![o.clone()],
            next_id: Some(6),
            durable: true,
            ..WriteBatch::default()
        })
        .await
        .expect("write");

    let snap = store.load_world().await.expect("load").expect("non-empty");
    assert_eq!(snap.objects.len(), 1);
    assert_eq!(
        snap.objects[0], o,
        "Obj must survive the store path exactly"
    );
}

// ---------------------------------------------------------------------
// Module persistence (`WriteBatch::module_ops` + the module read API)
// ---------------------------------------------------------------------

fn json_set(ns: &str, key: &str, value: serde_json::Value) -> ModuleOp {
    ModuleOp {
        ns: ns.into(),
        op: ModuleWrite::JsonSet {
            key: key.into(),
            value,
        },
    }
}

fn mod_del(ns: &str, key: &str) -> ModuleOp {
    ModuleOp {
        ns: ns.into(),
        op: ModuleWrite::Del { key: key.into() },
    }
}

fn stream_append(ns: &str, key: &str, value: serde_json::Value, maxlen: u32) -> ModuleOp {
    ModuleOp {
        ns: ns.into(),
        op: ModuleWrite::StreamAppend {
            key: key.into(),
            value,
            maxlen,
        },
    }
}

/// Write a batch carrying only module ops.
async fn write_module_ops(store: &dyn ObjectStore, serial: u64, module_ops: Vec<ModuleOp>) {
    store
        .write(WriteBatch {
            serial,
            module_ops,
            ..WriteBatch::default()
        })
        .await
        .expect("module-op batch");
}

/// Parse a stream id (`ms-seq` on Redis, `N-0` in memory) for order
/// comparison — string comparison would put `"10-0"` before `"9-0"`.
fn parse_stream_id(id: &str) -> (u64, u64) {
    let (major, minor) = id.split_once('-').expect("stream id has a dash");
    (
        major.parse().expect("numeric id major"),
        minor.parse().expect("numeric id minor"),
    )
}

/// A batch with an invalid module op (a `:` in the namespace would breach
/// namespace isolation) is refused as a unit: the object upsert, the valid
/// module op, and `next_id` riding the same batch must all be discarded.
pub async fn module_batch_atomicity(store: &dyn ObjectStore) {
    let (seed, _) = minimal_seed_batch(1);
    store.write(seed).await.expect("seed");

    let poisoned = WriteBatch {
        serial: 2,
        upserts: vec![thing(2, "alpha")],
        module_ops: vec![
            json_set("m", "good", serde_json::json!({"ok": true})),
            json_set("bad:ns", "k", serde_json::json!(1)),
        ],
        next_id: Some(3),
        ..WriteBatch::default()
    };
    let err = store.write(poisoned).await.expect_err("bad ns must refuse");
    assert!(
        matches!(err, StoreError::Invalid(_)),
        "expected Invalid, got {err:?}"
    );

    let snap = store.load_world().await.expect("load").expect("non-empty");
    assert_eq!(
        snap.next_id, 2,
        "next_id from the refused batch must not land"
    );
    assert_eq!(
        snap.objects.len(),
        2,
        "upsert from the refused batch must not land"
    );
    assert_eq!(
        store.module_json_get("m", "good").await.expect("get"),
        None,
        "valid module op from the refused batch must not land"
    );

    // Empty namespaces and keys are refused the same way.
    for op in [
        json_set("", "k", serde_json::json!(1)),
        json_set("m", "", serde_json::json!(1)),
    ] {
        let err = store
            .write(WriteBatch {
                serial: 3,
                module_ops: vec![op],
                ..WriteBatch::default()
            })
            .await
            .expect_err("empty ns/key must refuse");
        assert!(matches!(err, StoreError::Invalid(_)));
    }

    // The corrected batch applies object writes and module ops together.
    store
        .write(WriteBatch {
            serial: 4,
            upserts: vec![thing(2, "alpha")],
            module_ops: vec![json_set("m", "good", serde_json::json!({"ok": true}))],
            next_id: Some(3),
            ..WriteBatch::default()
        })
        .await
        .expect("corrected batch");
    let snap = store.load_world().await.expect("load").expect("non-empty");
    assert_eq!(snap.next_id, 3);
    assert_eq!(snap.objects.len(), 3);
    assert_eq!(
        store.module_json_get("m", "good").await.expect("get"),
        Some(serde_json::json!({"ok": true}))
    );
}

/// KV set / overwrite / delete / get / list, including namespace isolation:
/// `mod:a` and `mod:ab` share a string prefix, and keys may contain `:` —
/// neither may bleed into the other's listing.
pub async fn module_kv_round_trip(store: &dyn ObjectStore) {
    assert_eq!(
        store.module_json_get("a", "missing").await.expect("get"),
        None,
        "unknown key must be None"
    );

    write_module_ops(
        store,
        1,
        vec![
            json_set("a", "k1", serde_json::json!({"v": 1})),
            json_set("a", "k2", serde_json::json!([1, 2, 3])),
            json_set("a", "k:colon", serde_json::json!("colons allowed in keys")),
            json_set("ab", "k1", serde_json::json!({"other": "namespace"})),
        ],
    )
    .await;

    assert_eq!(
        store.module_json_get("a", "k1").await.expect("get"),
        Some(serde_json::json!({"v": 1}))
    );
    assert_eq!(
        store.module_json_get("ab", "k1").await.expect("get"),
        Some(serde_json::json!({"other": "namespace"})),
        "same key name in another namespace is a different doc"
    );
    assert_eq!(
        store.module_json_get("a", "k:colon").await.expect("get"),
        Some(serde_json::json!("colons allowed in keys"))
    );

    // Overwrite in a later batch: last write wins.
    write_module_ops(
        store,
        2,
        vec![json_set("a", "k1", serde_json::json!({"v": 2}))],
    )
    .await;
    assert_eq!(
        store.module_json_get("a", "k1").await.expect("get"),
        Some(serde_json::json!({"v": 2}))
    );

    // Delete k2; the listing then shows exactly a's surviving docs — and
    // nothing from `ab`, despite the shared prefix.
    write_module_ops(store, 3, vec![mod_del("a", "k2")]).await;
    assert_eq!(store.module_json_get("a", "k2").await.expect("get"), None);

    let mut listed = store.module_json_list("a").await.expect("list a");
    listed.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        listed,
        vec![
            ("k1".to_string(), serde_json::json!({"v": 2})),
            (
                "k:colon".to_string(),
                serde_json::json!("colons allowed in keys")
            ),
        ],
        "listing must not leak mod:ab keys into mod:a"
    );
    let listed = store.module_json_list("ab").await.expect("list ab");
    assert_eq!(
        listed,
        vec![("k1".to_string(), serde_json::json!({"other": "namespace"}))]
    );
}

/// Stream append / trim / range: oldest-first ordering, ids strictly
/// increasing across batches, `count_latest` picking the newest entries,
/// and the `MAXLEN ~` cap (approximate: at least `maxlen` survive, and
/// whatever survives is exactly a suffix of what was appended).
pub async fn module_stream_ordering(store: &dyn ObjectStore) {
    // Appends spread over several batches: ids must stay monotonic.
    write_module_ops(
        store,
        1,
        vec![
            stream_append("s", "log", serde_json::json!({"n": 1}), 100),
            stream_append("s", "log", serde_json::json!({"n": 2}), 100),
        ],
    )
    .await;
    write_module_ops(
        store,
        2,
        vec![
            stream_append("s", "log", serde_json::json!({"n": 3}), 100),
            stream_append("s", "log", serde_json::json!({"n": 4}), 100),
            stream_append("s", "log", serde_json::json!({"n": 5}), 100),
        ],
    )
    .await;

    let all = store.stream_range("s", "log", 100).await.expect("range");
    let values: Vec<i64> = all.iter().map(|(_, v)| v["n"].as_i64().unwrap()).collect();
    assert_eq!(values, [1, 2, 3, 4, 5], "oldest-first, append order");
    let ids: Vec<(u64, u64)> = all.iter().map(|(id, _)| parse_stream_id(id)).collect();
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "ids must be strictly increasing: {ids:?}"
    );

    // count_latest picks the newest entries but returns them oldest-first.
    let last_two = store.stream_range("s", "log", 2).await.expect("range 2");
    let values: Vec<i64> = last_two
        .iter()
        .map(|(_, v)| v["n"].as_i64().unwrap())
        .collect();
    assert_eq!(values, [4, 5]);

    // count 0 and a missing stream are both empty, not errors.
    assert!(
        store
            .stream_range("s", "log", 0)
            .await
            .expect("count 0")
            .is_empty()
    );
    assert!(
        store
            .stream_range("s", "nothing", 8)
            .await
            .expect("missing")
            .is_empty()
    );

    // Trimming: 5 appends against maxlen 2. `MAXLEN ~` guarantees at least
    // the newest `maxlen` survive; the in-memory store trims exactly, Redis
    // may lazily retain more — either way the survivors are a suffix.
    for i in 1..=5 {
        write_module_ops(
            store,
            10 + i,
            vec![stream_append("s", "capped", serde_json::json!(i), 2)],
        )
        .await;
    }
    let capped = store.stream_range("s", "capped", 100).await.expect("range");
    let values: Vec<i64> = capped.iter().map(|(_, v)| v.as_i64().unwrap()).collect();
    assert!(
        (2..=5).contains(&values.len()),
        "between maxlen and all entries survive, got {values:?}"
    );
    let expected_suffix: Vec<i64> = (1..=5).skip(5 - values.len()).collect();
    assert_eq!(
        values, expected_suffix,
        "survivors must be the newest suffix"
    );
}

/// Listing an untouched namespace yields an empty vec, and a namespace
/// holding only a stream lists no JSON docs (the stream key is skipped,
/// not an error).
pub async fn module_empty_namespace_list(store: &dyn ObjectStore) {
    assert_eq!(
        store.module_json_list("nothing").await.expect("list"),
        vec![],
        "untouched namespace must list empty"
    );

    write_module_ops(
        store,
        1,
        vec![stream_append("streamy", "log", serde_json::json!("x"), 4)],
    )
    .await;
    assert_eq!(
        store.module_json_list("streamy").await.expect("list"),
        vec![],
        "a stream-only namespace must list no JSON docs"
    );
    // ... while the stream itself is intact.
    assert_eq!(
        store
            .stream_range("streamy", "log", 4)
            .await
            .expect("range")
            .len(),
        1
    );
}
