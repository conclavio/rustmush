//! Shared behavior suite: every [`ObjectStore`] implementation must pass
//! these, so the in-memory fake proves the exact semantics the Redis store
//! ships with.
//!
//! Each function takes an **empty** store and panics (via `assert!`) on any
//! contract violation. The unit tests run them against [`crate::InMemoryStore`]
//! unconditionally; CI runs the explicitly ignored `tests/redis_suite.rs`
//! wrappers against a live disposable Redis with `REDIS_URL` set.
//!
//! This module is `#[doc(hidden)] pub` only so integration tests can reach
//! it; it is not API.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mush_core::{
    AccountId, Attr, AttrFlags, AttrValue, AuthoritativeBundle, AuthoritativeBundleDraft,
    AuthoritativeKey, CanonicalHash, CanonicalId, CommitToken, DefinitionDescriptor, DefinitionId,
    DefinitionKind, DefinitionRegistryBuilder, DurableOutboxIntent, Flags, GOD, IdempotencyKey,
    IdempotencyScope, LIMBO, Ledger, LockExpr, LockType, Obj, ObjId, ObjKind, OwnerId, Powers,
    PreparedCausalRef, PrincipalId, ReplayWrite, RequestUid, ResolutionKeyRing, ResolutionSecret,
    RulesValue, ScopedIdempotencyKey, World,
};

use crate::migrate::{Migration, run_migrations};
use crate::store::{AuthoritativeStore, ObjectStore};
use crate::types::{
    AccountRec, AuthoritativeOutboxAttemptResult, AuthoritativeRetentionUsage,
    AuthoritativeScanCursor, BoundIdempotencyStatus, BundleStatus, IdempotencyStatus, ModuleOp,
    ModuleWrite, SshKeyRec, StoreError, TokenRecord, WriteBatch,
};

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
        id: None,
        name: "Nick".to_string(),
        argon2_hash: hash.to_string(),
        created: now_unix(),
        characters: vec![],
        ssh_keys: vec![],
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
            id: None,
            name: "Nick".to_string(),
            argon2_hash: "$argon2id$fake".to_string(),
            created,
            characters: vec![],
            ssh_keys: vec![],
        })
        .await
        .expect("create");
    assert!(
        matches!(
            store
                .create_account(AccountRec {
                    id: None,
                    name: "NICK".to_string(),
                    argon2_hash: "other".to_string(),
                    created,
                    characters: vec![],
                    ssh_keys: vec![],
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
            id: None,
            name: "Zoe".to_string(),
            argon2_hash: "h".to_string(),
            created,
            characters: vec![],
            ssh_keys: vec![],
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

/// `set_account_password` (`@newpassword`): replaces only the hash
/// (case-insensitive lookup, everything else untouched), reports `false`
/// for unknown accounts, and never disturbs the account's tokens itself
/// (the caller pairs it with `tokens_del_account`).
pub async fn account_password_update(store: &dyn ObjectStore) {
    assert!(
        !store
            .set_account_password("nobody", "$argon2id$new")
            .await
            .expect("unknown account is not an error"),
        "unknown account must report false"
    );

    let created = now_unix();
    store
        .create_account(AccountRec {
            id: None,
            name: "Nick".to_string(),
            argon2_hash: "$argon2id$old".to_string(),
            created,
            characters: vec![],
            ssh_keys: vec![],
        })
        .await
        .expect("create");
    store
        .link_character("nick", ObjId(7), "Nicky")
        .await
        .expect("link");

    assert!(
        store
            .set_account_password("NICK", "$argon2id$new")
            .await
            .expect("update"),
        "existing account must report true (case-insensitive)"
    );
    let rec = store
        .get_account("nick")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.argon2_hash, "$argon2id$new");
    assert_eq!(rec.name, "Nick", "display case untouched");
    assert_eq!(rec.created, created, "created untouched");
    assert_eq!(rec.characters.len(), 1, "characters untouched");
}

/// `set_account_ssh_keys` (`@sshkey/add|del`): whole-array replace that
/// round-trips (set, then replace with a different array), reports `false`
/// for an unknown account (never an error), and leaves the rest of the
/// account doc untouched. The field is additive `#[serde(default)]`, so a
/// store whose account doc lacks `ssh_keys` still deserializes and is
/// writable (the Redis half proves the missing-`$.ssh_keys`-path case
/// directly in `tests/redis_suite.rs`).
pub async fn account_ssh_keys_update(store: &dyn ObjectStore) {
    let key = |fp: &str| SshKeyRec {
        openssh: format!("ssh-ed25519 AAAA{fp} nick@box"),
        fingerprint: format!("SHA256:{fp}"),
        added: 1_700_000_000,
    };

    assert!(
        !store
            .set_account_ssh_keys("nobody", &[key("aaa")])
            .await
            .expect("unknown account is not an error"),
        "unknown account must report false"
    );

    let created = now_unix();
    store
        .create_account(AccountRec {
            id: None,
            name: "Nick".to_string(),
            argon2_hash: "$argon2id$keep".to_string(),
            created,
            characters: vec![],
            ssh_keys: vec![],
        })
        .await
        .expect("create");
    store
        .link_character("nick", ObjId(7), "Nicky")
        .await
        .expect("link");

    // First set: two keys (case-insensitive account lookup).
    let first = vec![key("one"), key("two")];
    assert!(
        store
            .set_account_ssh_keys("NICK", &first)
            .await
            .expect("set"),
        "existing account must report true"
    );
    let rec = store
        .get_account("nick")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.ssh_keys, first, "keys round-trip");
    assert_eq!(rec.argon2_hash, "$argon2id$keep", "hash untouched");
    assert_eq!(rec.name, "Nick", "display case untouched");
    assert_eq!(rec.created, created, "created untouched");
    assert_eq!(rec.characters.len(), 1, "characters untouched");

    // Replace: a different, single-key array wholly supplants the first.
    let second = vec![key("three")];
    assert!(
        store
            .set_account_ssh_keys("nick", &second)
            .await
            .expect("replace"),
    );
    let rec = store
        .get_account("nick")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.ssh_keys, second, "replace is a whole-array swap");
    assert_eq!(rec.characters.len(), 1, "characters still untouched");

    // Clear: an empty slice empties the array.
    assert!(
        store
            .set_account_ssh_keys("nick", &[])
            .await
            .expect("clear")
    );
    let rec = store
        .get_account("nick")
        .await
        .expect("get")
        .expect("exists");
    assert!(rec.ssh_keys.is_empty(), "empty slice clears the keys");
}

/// E7 cut 5 B1: the immutable account id round-trips through the store and is
/// preserved across every mutation, so a rename (there is no rename verb; this
/// pins the forward guarantee) or a password/ssh/character change never
/// disturbs it. The id is the caller's to mint (`AccountId::mint`, deterministic
/// over `(lowercased name, created)`); the store round-trips it verbatim.
pub async fn account_id_round_trips_and_is_immutable(store: &dyn ObjectStore) {
    let created = now_unix();
    let id = AccountId::mint("Nick", created).expect("mint");
    store
        .create_account(AccountRec {
            id: Some(id),
            name: "Nick".to_string(),
            argon2_hash: "$argon2id$fake".to_string(),
            created,
            characters: vec![],
            ssh_keys: vec![],
        })
        .await
        .expect("create");
    // The stored id is exactly the minted one, read back case-insensitively.
    let rec = store
        .get_account("NICK")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.id, Some(id), "the stored id round-trips");
    // Re-minting from the SAME (name, created) reproduces the id — the forward
    // rename-stability guarantee: the id does not depend on the current name,
    // only on the immutable (lowercased name, created) it was minted from.
    assert_eq!(AccountId::mint("nick", created), Ok(id));
    // Every later mutation leaves the id untouched.
    store
        .set_account_password("nick", "$argon2id$new")
        .await
        .expect("password");
    store
        .link_character("nick", ObjId(7), "Nicky")
        .await
        .expect("link");
    store
        .set_account_ssh_keys(
            "nick",
            &[SshKeyRec {
                openssh: "ssh-ed25519 AAAAkey nick@box".to_string(),
                fingerprint: "SHA256:key".to_string(),
                added: created,
            }],
        )
        .await
        .expect("ssh");
    let rec = store
        .get_account("nick")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.id, Some(id), "the id survives every mutation");
    assert_eq!(rec.argon2_hash, "$argon2id$new");
    assert_eq!(rec.characters.len(), 1);
    assert_eq!(rec.ssh_keys.len(), 1);
}

/// E7 cut 5 B1: a pre-id account document decodes with `id == None`
/// (`#[serde(default)]`), and the boot back-fill's mint is deterministic and
/// idempotent — writing the mint for a `None` id, then re-minting, yields the
/// same id, so re-running the back-fill is a no-op. This pins the migration the
/// server performs at boot; the store itself never mints.
pub async fn account_id_backfill_is_idempotent(store: &dyn ObjectStore) {
    let created = now_unix();
    // A pre-id document: `id` omitted, decoded as None via serde default.
    store
        .create_account(AccountRec {
            id: None,
            name: "Nick".to_string(),
            argon2_hash: "$argon2id$fake".to_string(),
            created,
            characters: vec![],
            ssh_keys: vec![],
        })
        .await
        .expect("create");
    let rec = store
        .get_account("nick")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.id, None, "a pre-id document decodes with a None id");
    // The back-fill mint is a pure function of (name, created): deterministic
    // and idempotent — the second run computes the identical id.
    let minted = AccountId::mint(&rec.name, rec.created).expect("mint");
    assert_eq!(AccountId::mint(&rec.name, rec.created), Ok(minted));
    // A different creation second is a different account and a different id.
    assert_ne!(AccountId::mint(&rec.name, rec.created + 1), Ok(minted));
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
    o.behaviors.insert(
        mush_core::BehaviorId::new("tactics.unit").unwrap(),
        mush_core::BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 1_700_000_500,
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
// Web session tokens (`token:{hash}` keys)
//
// Timing note: like `ban_expiry`, these use wide margins (±1 h) around the
// system clock so they hold against a live Redis across a WAN tunnel with
// modest clock skew.
// ---------------------------------------------------------------------

fn token(account: &str, created: i64, expires: i64) -> TokenRecord {
    TokenRecord {
        account: account.to_string(),
        created,
        expires,
    }
}

/// Put → get round trip, overwrite (last write wins), expired-at-put and
/// expired-at-get both read as `None`, delete semantics, and input
/// validation (empty hash refused).
pub async fn token_lifecycle(store: &dyn ObjectStore) {
    let now = now_unix();
    assert_eq!(
        store.token_get("cafe01").await.expect("get"),
        None,
        "unknown token must be None"
    );
    assert!(
        matches!(
            store
                .token_put(String::new(), token("nick", now, now + 3600))
                .await,
            Err(StoreError::Invalid(_))
        ),
        "empty hash must be refused"
    );

    // Live token round-trips exactly.
    let rec = token("Nick", now, now + 3600);
    store
        .token_put("cafe01".into(), rec.clone())
        .await
        .expect("put");
    assert_eq!(store.token_get("cafe01").await.expect("get"), Some(rec));

    // Last write wins: a re-put replaces the record.
    let rec2 = token("Nick", now + 1, now + 7200);
    store
        .token_put("cafe01".into(), rec2.clone())
        .await
        .expect("re-put");
    assert_eq!(store.token_get("cafe01").await.expect("get"), Some(rec2));

    // A put with expires in the past is a delete.
    store
        .token_put("cafe01".into(), token("Nick", now, now - 10))
        .await
        .expect("lapsed put");
    assert_eq!(
        store.token_get("cafe01").await.expect("get"),
        None,
        "a token put already-expired must be gone"
    );

    // An expired token reads as None (lazy delete path).
    store
        .token_put("dead01".into(), token("Nick", now - 100, now - 10))
        .await
        .expect("expired put");
    assert_eq!(store.token_get("dead01").await.expect("get"), None);

    // Delete: true for a live token, false thereafter and for unknowns.
    store
        .token_put("gone01".into(), token("Nick", now, now + 3600))
        .await
        .expect("put");
    assert!(store.token_del("gone01").await.expect("del"));
    assert!(!store.token_del("gone01").await.expect("re-del"));
    assert!(!store.token_del("never01").await.expect("del unknown"));
    assert_eq!(store.token_get("gone01").await.expect("get"), None);
}

/// Sliding-TTL semantics of `token_touch`: extends a live token (record
/// field and store TTL together), reports `false` for missing/expired
/// tokens without resurrecting them, and deletes on a past `new_expires`.
pub async fn token_sliding_ttl(store: &dyn ObjectStore) {
    let now = now_unix();
    assert!(
        !store
            .token_touch("ghost01", now + 3600)
            .await
            .expect("touch"),
        "touching an unknown token is false"
    );

    store
        .token_put("slide01".into(), token("Zoe", now, now + 3600))
        .await
        .expect("put");
    assert!(
        store
            .token_touch("slide01", now + 7200)
            .await
            .expect("touch")
    );
    let rec = store
        .token_get("slide01")
        .await
        .expect("get")
        .expect("still live");
    assert_eq!(rec.expires, now + 7200, "touch must move expires");
    assert_eq!(rec.account, "Zoe", "identity fields survive the touch");
    assert_eq!(rec.created, now);

    // Touching an expired token must not resurrect it.
    store
        .token_put("stale01".into(), token("Zoe", now - 100, now - 10))
        .await
        .expect("expired put");
    assert!(
        !store
            .token_touch("stale01", now + 3600)
            .await
            .expect("touch"),
        "an expired token is missing; a touch never resurrects"
    );
    assert_eq!(store.token_get("stale01").await.expect("get"), None);

    // Sliding into the past deletes (and reports the token was live).
    assert!(store.token_touch("slide01", now - 10).await.expect("touch"));
    assert_eq!(store.token_get("slide01").await.expect("get"), None);
}

/// `tokens_del_account`: deletes every live token of one account
/// (case-insensitive), leaves other accounts' tokens intact, and reports
/// the live-token count.
pub async fn token_del_account(store: &dyn ObjectStore) {
    let now = now_unix();
    store
        .token_put("nick-a".into(), token("Nick", now, now + 3600))
        .await
        .expect("put");
    store
        .token_put("nick-b".into(), token("Nick", now, now + 7200))
        .await
        .expect("put");
    store
        .token_put("nick-dead".into(), token("Nick", now - 100, now - 10))
        .await
        .expect("expired put");
    store
        .token_put("zoe-a".into(), token("Zoe", now, now + 3600))
        .await
        .expect("put");

    let removed = store.tokens_del_account("nIcK").await.expect("del account");
    assert_eq!(removed, 2, "both live Nick tokens, not the expired one");
    assert_eq!(store.token_get("nick-a").await.expect("get"), None);
    assert_eq!(store.token_get("nick-b").await.expect("get"), None);
    assert!(
        store.token_get("zoe-a").await.expect("get").is_some(),
        "other accounts' tokens must survive"
    );

    // Idempotent: nothing left to delete.
    assert_eq!(store.tokens_del_account("nick").await.expect("re-del"), 0);
    // An account with no tokens at all is simply zero.
    assert_eq!(store.tokens_del_account("nobody").await.expect("none"), 0);
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

/// The store itself—not only the server driver—rejects every legacy module
/// operation aimed at the authoritative typed-state keyspace. The refusal is
/// atomic with unrelated object/module/allocator writes in the same batch.
pub async fn authoritative_module_prefix_is_store_owned(store: &dyn ObjectStore) {
    let (seed, _) = minimal_seed_batch(1);
    store.write(seed).await.expect("seed");

    for reserved in [
        json_set(
            "toy",
            "action-state/v1/focus/1",
            serde_json::json!({"amount": 2}),
        ),
        mod_del("toy", "action-state/v1/focus/1"),
        stream_append(
            "toy",
            "action-state/v1/focus/1",
            serde_json::json!({"amount": 2}),
            10,
        ),
    ] {
        let error = store
            .write(WriteBatch {
                serial: 2,
                upserts: vec![thing(2, "must-not-land")],
                module_ops: vec![
                    json_set("toy", "ordinary", serde_json::json!({"ok": true})),
                    reserved,
                ],
                next_id: Some(3),
                ..WriteBatch::default()
            })
            .await
            .expect_err("legacy write into authoritative prefix must fail");
        assert!(
            matches!(error, StoreError::Invalid(ref message) if message.contains("owned by authoritative action persistence")),
            "unexpected reserved-prefix refusal: {error:?}"
        );
        let snapshot = store.load_world().await.unwrap().unwrap();
        assert_eq!(snapshot.next_id, 2);
        assert_eq!(snapshot.objects.len(), 2);
        assert_eq!(
            store.module_json_get("toy", "ordinary").await.unwrap(),
            None
        );
        assert_eq!(
            store
                .module_json_get("toy", "action-state/v1/focus/1")
                .await
                .unwrap(),
            None
        );
    }
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
/// J1: `module_docs` splits a namespace into decoded ordinary documents and
/// ledger chunks as text, agrees with `module_json_list` on the whole, and
/// keeps the namespace boundary.
pub async fn module_docs_splits_the_ledger_family(store: &dyn ObjectStore) {
    let chunk = serde_json::json!({
        "chunk": 0, "sealed": false, "entries": [{ "seq": 0, "value": { "n": 1 } }]
    });
    write_module_ops(
        store,
        1,
        vec![
            json_set("a", "k1", serde_json::json!({ "v": 1 })),
            json_set("a", "log:j:0", chunk.clone()),
            json_set("ab", "log:j:0", serde_json::json!({ "other": "namespace" })),
        ],
    )
    .await;

    let docs = store.module_docs("a").await.expect("split listing");
    assert_eq!(
        docs.docs,
        vec![("k1".to_string(), serde_json::json!({ "v": 1 }))]
    );
    assert_eq!(
        docs.ledger.len(),
        1,
        "the other namespace's chunk is not ours"
    );
    assert_eq!(docs.ledger[0].0, "log:j:0");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&docs.ledger[0].1).expect("chunk text is JSON"),
        chunk,
        "the text is the stored document"
    );

    let mut flat = store.module_json_list("a").await.expect("flat listing");
    flat.sort_by(|x, y| x.0.cmp(&y.0));
    let mut from_split = docs.into_json_list().expect("split decodes");
    from_split.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(flat, from_split, "the two listings agree on the whole");

    assert_eq!(
        store.module_docs("zz").await.expect("unknown namespace"),
        crate::ModuleDocs::default()
    );
}

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

// -------------------------------------------------------------------------
// Shared authoritative-store contract
// -------------------------------------------------------------------------

pub(crate) fn authoritative_bundle(token_byte: u8, request_byte: u8) -> AuthoritativeBundle {
    authoritative_bundle_with(token_byte, request_byte, 0, vec![], false, vec![])
}

fn authoritative_bundle_with(
    token_byte: u8,
    request_byte: u8,
    epoch: u64,
    durable_outbox: Vec<DurableOutboxIntent>,
    with_resolution: bool,
    extra_writes: Vec<ReplayWrite>,
) -> AuthoritativeBundle {
    let owner = OwnerId::new("store_contract").expect("contract owner");
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").expect("kind"),
                DefinitionId::new("brace").expect("definition"),
                1,
                serde_json::json!({}),
            )
            .expect("action descriptor"),
        )
        .expect("register action");
    let outcome = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").expect("kind"),
                DefinitionId::new("braced").expect("definition"),
                1,
                serde_json::json!({}),
            )
            .expect("outcome descriptor"),
        )
        .expect("register outcome");
    let request = RequestUid::from_bytes([request_byte; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let token = CommitToken::from_bytes([token_byte; 16]);
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: owner.clone(),
            namespace: CanonicalId::new("brace").expect("namespace"),
        },
        epoch,
        IdempotencyKey::Request(request),
    );
    let request_hash = CanonicalHash::from_bytes([3; 32]);
    let outcome_value =
        RulesValue::new(outcome.clone(), serde_json::json!({ "ok": true })).expect("outcome value");
    let definition_refs = vec![action.clone(), outcome.clone()];
    let resolution = with_resolution.then(|| {
        let mut rng = ResolutionKeyRing::new(
            ResolutionSecret::new(1, [0x51; 32]).expect("resolution secret"),
            [],
        )
        .expect("resolution key ring")
        .start(
            &idempotency,
            &CanonicalId::new("store_contract_resolution").expect("resolution domain"),
            request_hash,
        )
        .expect("start resolution");
        rng.draw_bounded_u64(&CanonicalId::new("effect_duration").expect("draw label"), 6)
            .expect("draw resolution");
        rng.into_prepared(
            definition_refs.clone(),
            RulesValue::new(outcome.clone(), serde_json::json!({ "bonus": 1 }))
                .expect("modifier value"),
            outcome_value.clone(),
        )
        .expect("prepare resolution")
        .bind(token)
    });
    let mut writes = vec![
        ReplayWrite::ObjectSet {
            object: Obj::new(ObjId(2), ObjKind::Thing, "guard", GOD, 0),
        },
        ReplayWrite::ObjectAllocatorSet { next_id: 3 },
        ReplayWrite::ModuleJsonSet {
            namespace: owner.clone(),
            key: AuthoritativeKey::parse_canonical("action-state/v1/guarded/2").expect("state key"),
            value: serde_json::json!({ "amount": 1 }),
        },
    ];
    writes.extend(extra_writes);
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request,
        idempotency,
        ingress_hash: CanonicalHash::from_bytes([request_byte; 32]),
        request_hash,
        plan_hash: CanonicalHash::from_bytes([4; 32]),
        action: action.clone(),
        provider: owner.clone(),
        cause: PreparedCausalRef::new(request, None, 0).expect("root cause"),
        definitions: definition_refs,
        writes,
        outcome: outcome_value,
        resolution,
        semantic_events: vec![
            mush_core::AuthoritativeSemanticEvent::root(
                PreparedCausalRef::new(request, None, 0).expect("root cause"),
                &action,
                RulesValue::new(outcome, serde_json::json!({ "ok": true })).expect("event value"),
                vec![principal.clone()],
            )
            .expect("root event"),
        ],
        eligible_witnesses: vec![principal],
        durable_outbox,
    })
    .expect("authoritative contract bundle")
}

async fn seed_authoritative_world(store: &dyn AuthoritativeStore) {
    let (seed, _) = minimal_seed_batch(1);
    store.write(seed).await.expect("seed minimal world");
}

/// Exact reservation, both discovery paths, replay/finalization, compact
/// checkpoint, and exact retry have identical semantics on every backend.
pub async fn authoritative_exact_lifecycle(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let bundle = authoritative_bundle(11, 21);
    let token = bundle.token();
    let hash = bundle.bundle_hash();

    assert_eq!(
        store
            .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
            .await
            .expect("empty idempotency lookup"),
        IdempotencyStatus::Vacant
    );
    assert_eq!(
        store
            .bound_authoritative_idempotency_status(bundle.idempotency())
            .await
            .expect("empty bound idempotency lookup"),
        BoundIdempotencyStatus::Vacant
    );
    let reservation = store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("reserve intent");
    assert!(!reservation.resumed);
    assert_eq!(reservation.token, token);
    assert_eq!(reservation.bundle_hash, hash);
    assert_eq!(
        store
            .bound_authoritative_idempotency_status(bundle.idempotency())
            .await
            .expect("pending bound idempotency lookup"),
        BoundIdempotencyStatus::PendingExact {
            token,
            bundle_hash: hash,
        }
    );
    assert_eq!(
        store
            .pending_authoritative_intents()
            .await
            .expect("pending accelerator")
            .len(),
        1
    );

    let mut cursor: Option<AuthoritativeScanCursor> = None;
    let mut discovered = Vec::new();
    loop {
        let page = store
            .scan_authoritative_intents(cursor, 1)
            .await
            .expect("scan full intent keyspace");
        discovered.extend(page.intents);
        let Some(next) = page.next_cursor else {
            break;
        };
        cursor = Some(next);
    }
    assert_eq!(discovered, vec![bundle.clone()]);

    let receipt = store
        .reconcile_authoritative(token, hash)
        .await
        .expect("reconcile intent");
    assert!(receipt.matches_bundle(&bundle));
    assert_eq!(receipt.semantic_events(), bundle.semantic_events());
    assert_eq!(
        store.authoritative_status(token, hash).await.unwrap(),
        BundleStatus::FinalizedExact(Box::new(receipt.clone()))
    );
    assert_eq!(
        store
            .bound_authoritative_idempotency_status(bundle.idempotency())
            .await
            .expect("finalized bound idempotency lookup"),
        BoundIdempotencyStatus::FinalizedExact(Box::new(receipt.clone()))
    );
    assert!(
        store
            .pending_authoritative_intents()
            .await
            .unwrap()
            .is_empty()
    );

    let checkpoint = store
        .checkpoint_authoritative(token, hash)
        .await
        .expect("checkpoint finalized intent");
    assert_eq!(checkpoint.receipt, receipt);
    assert_eq!(
        checkpoint.receipt.semantic_events(),
        bundle.semantic_events()
    );
    assert!(!checkpoint.resumed);
    assert_eq!(
        store
            .bound_authoritative_idempotency_status(bundle.idempotency())
            .await
            .expect("checkpointed bound idempotency lookup"),
        BoundIdempotencyStatus::FinalizedExact(Box::new(receipt.clone()))
    );
    assert!(
        store
            .scan_authoritative_intents(None, 1)
            .await
            .unwrap()
            .intents
            .is_empty()
    );
    assert!(
        store
            .reserve_authoritative_intent(bundle)
            .await
            .expect("join exact compact retry")
            .resumed
    );
    assert!(
        store
            .checkpoint_authoritative(token, hash)
            .await
            .expect("join checkpoint retry")
            .resumed
    );
}

/// A mixed object/allocator/typed-document/Ledger bundle materializes one
/// exact sealed chunk and retains it after the full replay intent checkpoints.
/// This is shared by memory and Redis so K8.0 cannot become backend-specific.
pub async fn authoritative_mixed_ledger_lifecycle(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let owner = OwnerId::new("store_contract").expect("contract owner");
    let ledger = Ledger::with_chunk_target(0);
    let prepared = ledger
        .prepare_append("economy", serde_json::json!({ "amount": 7 }))
        .expect("prepare sealed Ledger append");
    let replay = prepared
        .authoritative_replay_write(owner.clone())
        .expect("build authoritative Ledger replay");
    let ReplayWrite::LedgerChunkSet { write } = &replay else {
        panic!("prepared Ledger append returned another replay kind")
    };
    let expected = serde_json::to_value(write.chunk()).expect("Ledger chunk JSON");
    let seal = write.seal().expect("target-zero append seals");
    let bundle = authoritative_bundle_with(20, 30, 0, vec![], false, vec![replay]);
    assert_eq!(bundle.ledger_seals(), &[seal]);
    let token = bundle.token();
    let hash = bundle.bundle_hash();

    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("reserve mixed Ledger intent");
    assert_eq!(
        store
            .module_json_get(owner.as_str(), "log:economy:0")
            .await
            .expect("pre-materialization Ledger read"),
        None
    );
    let receipt = store
        .reconcile_authoritative(token, hash)
        .await
        .expect("materialize mixed Ledger intent");
    assert!(receipt.matches_bundle(&bundle));
    assert_eq!(
        store
            .module_json_get(owner.as_str(), "log:economy:0")
            .await
            .expect("materialized Ledger read"),
        Some(expected.clone())
    );
    let checkpoint = store
        .checkpoint_authoritative(token, hash)
        .await
        .expect("checkpoint mixed Ledger intent");
    assert_eq!(checkpoint.receipt, receipt);
    assert_eq!(
        store
            .module_json_get(owner.as_str(), "log:economy:0")
            .await
            .expect("post-checkpoint Ledger read"),
        Some(expected)
    );
    assert!(
        store
            .reserve_authoritative_intent(bundle)
            .await
            .expect("join compact mixed Ledger retry")
            .resumed
    );
}

/// A resolution transcript is part of the exact authoritative identity and
/// survives intent release into the compact checkpoint receipt. This shared
/// witness exercises both the memory backend and the live Redis contract.
pub async fn authoritative_resolution_checkpoint(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let bundle = authoritative_bundle_with(18, 28, 0, vec![], true, vec![]);
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    let expected = bundle
        .resolution()
        .cloned()
        .expect("resolution-bearing fixture");

    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("reserve resolution-bearing intent");
    let receipt = store
        .reconcile_authoritative(token, hash)
        .await
        .expect("reconcile resolution-bearing intent");
    assert_eq!(receipt.resolution(), Some(&expected));
    assert!(receipt.matches_bundle(&bundle));

    let checkpoint = store
        .checkpoint_authoritative(token, hash)
        .await
        .expect("checkpoint resolution-bearing intent");
    assert_eq!(checkpoint.receipt.resolution(), Some(&expected));
    assert!(checkpoint.receipt.matches_bundle(&bundle));
    assert!(
        store
            .reserve_authoritative_intent(bundle)
            .await
            .expect("join compact resolution retry")
            .resumed
    );
    assert_eq!(
        store
            .checkpoint_authoritative(token, hash)
            .await
            .expect("resume compact resolution checkpoint")
            .receipt
            .resolution(),
        Some(&expected)
    );
}

/// Admission is enforced by the backend, not merely by a single coordinator
/// task, and is released only after the unresolved intent finalizes.
pub async fn authoritative_single_pending_admission(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let first = authoritative_bundle(12, 22);
    let second = authoritative_bundle(13, 23);
    store
        .reserve_authoritative_intent(first.clone())
        .await
        .expect("reserve first intent");
    assert!(matches!(
        store.reserve_authoritative_intent(second.clone()).await,
        Err(StoreError::AuthoritativeIntentInFlight { pending: 1 })
    ));
    store
        .reconcile_authoritative(first.token(), first.bundle_hash())
        .await
        .expect("finalize first intent");
    store
        .reserve_authoritative_intent(second)
        .await
        .expect("admit after first finalizes");
}

/// Quarantine consumes identity, releases full-intent capacity, rejects
/// replay, and permits only an exact evidence-backed restore.
pub async fn authoritative_quarantine_lifecycle(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let bundle = authoritative_bundle(14, 24);
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    let evidence_hash = CanonicalHash::from_bytes([0x1c; 32]);
    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("reserve quarantine candidate");
    let discarded = store
        .discard_quarantined_authoritative_intent(token, hash, evidence_hash)
        .await
        .expect("discard to quarantine tombstone");
    assert!(!discarded.resumed);
    assert_eq!(
        store.authoritative_status(token, hash).await.unwrap(),
        BundleStatus::QuarantinedExact(Box::new(discarded.quarantine.clone()))
    );
    assert_eq!(
        store
            .bound_authoritative_idempotency_status(bundle.idempotency())
            .await
            .expect("quarantined bound idempotency lookup"),
        BoundIdempotencyStatus::QuarantinedExact(Box::new(discarded.quarantine.clone()))
    );
    assert!(matches!(
        store.reserve_authoritative_intent(bundle.clone()).await,
        Err(StoreError::AuthoritativeIntentQuarantined {
            evidence_hash: observed
        }) if observed == evidence_hash
    ));
    assert!(
        store
            .discard_quarantined_authoritative_intent(token, hash, evidence_hash)
            .await
            .expect("join discard acknowledgement")
            .resumed
    );
    assert!(matches!(
        store
            .restore_quarantined_authoritative_intent(
                bundle.clone(),
                CanonicalHash::from_bytes([0x2c; 32]),
            )
            .await,
        Err(StoreError::AuthoritativeTokenConflict)
    ));
    assert!(
        !store
            .restore_quarantined_authoritative_intent(bundle, evidence_hash)
            .await
            .expect("restore exact evidence")
            .resumed
    );
    assert_eq!(
        store.authoritative_status(token, hash).await.unwrap(),
        BundleStatus::PendingExact
    );
}

/// Outbox state is finalization-gated, attempt and unroutable acknowledgement
/// loss is idempotent, and checkpoint remains blocked until terminal state.
pub async fn authoritative_outbox_lifecycle(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let intent = DurableOutboxIntent::new(
        0,
        OwnerId::new("store_contract").expect("owner"),
        CanonicalId::new("webhook").expect("adapter"),
        serde_json::json!({"event": "braced"}),
    )
    .expect("outbox intent");
    let payload_hash = intent.payload_hash();
    let bundle = authoritative_bundle_with(15, 25, 0, vec![intent], false, vec![]);
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    store
        .reserve_authoritative_intent(bundle)
        .await
        .expect("reserve outbox parent");
    assert!(
        store
            .pending_authoritative_outbox()
            .await
            .unwrap()
            .is_empty()
    );
    store
        .reconcile_authoritative(token, hash)
        .await
        .expect("finalize outbox parent");
    let pending = store.pending_authoritative_outbox().await.unwrap();
    assert_eq!(pending.len(), 1);
    let item_id = pending[0].item_id();
    let unroutable = store
        .note_authoritative_outbox_unroutable(
            item_id,
            payload_hash,
            0,
            "adapter unavailable".to_owned(),
        )
        .await
        .expect("record unroutable pass");
    assert_eq!(unroutable.unroutable_passes(), 1);
    assert_eq!(unroutable.attempts(), 0);
    assert_eq!(
        store
            .note_authoritative_outbox_unroutable(
                item_id,
                payload_hash,
                0,
                "adapter unavailable".to_owned(),
            )
            .await
            .expect("join unroutable acknowledgement"),
        unroutable
    );
    assert!(matches!(
        store.checkpoint_authoritative(token, hash).await,
        Err(StoreError::AuthoritativeOutboxPending)
    ));
    let attempt = store
        .begin_authoritative_outbox_attempt(item_id, payload_hash, 0)
        .await
        .expect("begin delivery attempt");
    assert_eq!(attempt.attempts(), 1);
    assert_eq!(
        store
            .begin_authoritative_outbox_attempt(item_id, payload_hash, 0)
            .await
            .expect("join attempt acknowledgement"),
        attempt
    );
    store
        .complete_authoritative_outbox_attempt(
            item_id,
            payload_hash,
            1,
            AuthoritativeOutboxAttemptResult::Delivered,
        )
        .await
        .expect("complete delivery");
    assert!(
        store
            .pending_authoritative_outbox()
            .await
            .unwrap()
            .is_empty()
    );
    store
        .complete_authoritative_outbox_attempt(
            item_id,
            payload_hash,
            1,
            AuthoritativeOutboxAttemptResult::Delivered,
        )
        .await
        .expect("join delivery acknowledgement");
    store
        .checkpoint_authoritative(token, hash)
        .await
        .expect("terminal outbox permits checkpoint");
}

/// Retirement advances a durable scope watermark, does not reopen compacted
/// identities, and remains blocked by pending or uncheckpointed epochs.
pub async fn authoritative_epoch_lifecycle(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let old = authoritative_bundle_with(16, 26, 0, vec![], false, vec![]);
    let scope = old.idempotency().scope().clone();
    store
        .reserve_authoritative_intent(old.clone())
        .await
        .expect("reserve old epoch");
    store
        .reconcile_authoritative(old.token(), old.bundle_hash())
        .await
        .expect("finalize old epoch");
    store
        .checkpoint_authoritative(old.token(), old.bundle_hash())
        .await
        .expect("checkpoint old epoch");
    assert_eq!(
        store
            .retire_authoritative_epochs(&scope, 1)
            .await
            .expect("retire old epoch")
            .retired_tombstones,
        1
    );
    assert!(matches!(
        store.reserve_authoritative_intent(old.clone()).await,
        Err(StoreError::AuthoritativeEpochRetired {
            requested_epoch: 0,
            minimum_accepted_epoch: 1,
        })
    ));
    assert_eq!(
        store.authoritative_bundle_status(&old).await.unwrap(),
        BundleStatus::Retired {
            minimum_accepted_epoch: 1
        }
    );
    assert_eq!(
        store
            .bound_authoritative_idempotency_status(old.idempotency())
            .await
            .expect("retired bound idempotency lookup"),
        BoundIdempotencyStatus::Retired {
            minimum_accepted_epoch: 1
        }
    );

    let current = authoritative_bundle_with(17, 27, 1, vec![], false, vec![]);
    store
        .reserve_authoritative_intent(current.clone())
        .await
        .expect("reserve current epoch");
    assert!(matches!(
        store.retire_authoritative_epochs(&scope, 2).await,
        Err(StoreError::AuthoritativeEpochPending { .. })
    ));
    store
        .reconcile_authoritative(current.token(), current.bundle_hash())
        .await
        .expect("finalize current epoch");
    assert!(matches!(
        store.retire_authoritative_epochs(&scope, 2).await,
        Err(StoreError::AuthoritativeEpochPending { .. })
    ));
    store
        .checkpoint_authoritative(current.token(), current.bundle_hash())
        .await
        .expect("checkpoint current epoch");
    store
        .retire_authoritative_epochs(&scope, 2)
        .await
        .expect("retire current epoch");
}

/// Token/hash and scoped-idempotency conflicts are closed identically by all
/// stores, including the hash-aware status path.
pub async fn authoritative_conflict_contract(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let first = authoritative_bundle(18, 28);
    store
        .reserve_authoritative_intent(first.clone())
        .await
        .expect("reserve first identity");
    assert!(matches!(
        store
            .reserve_authoritative_intent(authoritative_bundle(18, 29))
            .await,
        Err(StoreError::AuthoritativeTokenConflict)
    ));
    assert!(matches!(
        store
            .reserve_authoritative_intent(authoritative_bundle(19, 28))
            .await,
        Err(StoreError::AuthoritativeIdempotencyConflict)
    ));
    let wrong_hash = CanonicalHash::from_bytes([99; 32]);
    assert_eq!(
        store
            .authoritative_status(first.token(), wrong_hash)
            .await
            .expect("hash-aware status"),
        BundleStatus::Conflict {
            stored_bundle_hash: first.bundle_hash()
        }
    );
    assert!(matches!(
        store
            .reconcile_authoritative(first.token(), wrong_hash)
            .await,
        Err(StoreError::AuthoritativeTokenConflict)
    ));
}

/// Retention repair is guarded by membership generation so a stale complete
/// scan can never overwrite counters after another caller changes custody.
pub async fn authoritative_retention_generation_contract(store: &dyn AuthoritativeStore) {
    seed_authoritative_world(store).await;
    let stale_generation = store
        .authoritative_retention_generation()
        .await
        .expect("capture empty generation");
    let bundle = authoritative_bundle(20, 30);
    let usage = AuthoritativeRetentionUsage {
        count: 1,
        bytes: serde_json::to_string(&bundle)
            .expect("canonical bundle JSON")
            .len(),
    };
    store
        .reserve_authoritative_intent(bundle)
        .await
        .expect("change retained membership");
    assert!(matches!(
        store
            .repair_authoritative_retention(stale_generation, usage)
            .await,
        Err(StoreError::AuthoritativeRetentionContention)
    ));
    let current_generation = store
        .authoritative_retention_generation()
        .await
        .expect("capture current generation");
    assert!(
        !store
            .repair_authoritative_retention(current_generation, usage)
            .await
            .expect("exact counters are an idempotent no-op")
    );
}
