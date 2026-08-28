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
use crate::types::{AccountRec, StoreError, WriteBatch};

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
        deletes: vec![],
        next_id: Some(world.next_id()),
        durable: true,
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
            deletes: vec![],
            next_id: Some(4),
            durable: false,
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
            durable: false,
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
            deletes: vec![],
            next_id: Some(6),
            durable: true,
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
