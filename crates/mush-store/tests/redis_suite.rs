//! The shared behavior suite (plus Redis-only tests: index bootstrap +
//! search, native TTL bans) against a **live Redis 8**, gated on the
//! `REDIS_URL` env var.
//!
//! Without `REDIS_URL` every test skips cleanly with a note — the dev box
//! has no Redis; Linux CI runs a `redis:8` service container.
//!
//! **WARNING:** these tests delete every `obj:*`, `account:*`, `ban:*`,
//! `meta:*`, and `mod:*` key in the configured database between phases —
//! module data (channels, mail, jobs) is wiped along with the world. Point
//! `REDIS_URL` at a disposable database.
//!
//! Tests in this file share one database, so they serialize on a static
//! async mutex (cargo runs `#[tokio::test]`s in one process concurrently).

use mush_store::{ObjectStore, RedisStore, WriteBatch, suite};
use tokio::sync::Mutex;

static REDIS_LOCK: Mutex<()> = Mutex::const_new(());

/// Connect and wipe, or `None` (with a note) when `REDIS_URL` is unset
/// (or set but empty — `REDIS_URL= cargo test` must skip, not panic).
async fn fresh_store(test: &str) -> Option<RedisStore> {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipping {test}: REDIS_URL not set (no live Redis on this machine)");
        return None;
    };
    let store = RedisStore::connect(&url)
        .await
        .unwrap_or_else(|e| panic!("REDIS_URL set but connect failed: {e}"));
    store.wipe_namespaces().await.expect("wipe test namespaces");
    Some(store)
}

macro_rules! redis_gated {
    ($name:ident, $store:ident, $body:block) => {
        #[tokio::test]
        async fn $name() {
            let _serialize = REDIS_LOCK.lock().await;
            let Some($store) = fresh_store(stringify!($name)).await else {
                return;
            };
            $body
        }
    };
}

redis_gated!(empty_seed_load_round_trip, store, {
    suite::empty_seed_load_round_trip(&store).await;
});
redis_gated!(batch_atomicity, store, {
    suite::batch_atomicity(&store).await;
});
redis_gated!(next_id_persistence, store, {
    suite::next_id_persistence(&store).await;
});
redis_gated!(account_reserve_race, store, {
    suite::account_reserve_race(&store).await;
});
redis_gated!(account_lifecycle, store, {
    suite::account_lifecycle(&store).await;
});
redis_gated!(ban_expiry, store, {
    suite::ban_expiry(&store).await;
});
redis_gated!(migration_version_bump, store, {
    suite::migration_version_bump(&store).await;
});
redis_gated!(serde_obj_round_trip, store, {
    suite::serde_obj_round_trip(&store).await;
});

// Module persistence twins (M3): same shared suite functions as
// tests/memory_suite.rs, against live Redis. The wipe between phases
// covers `mod:*` too.
redis_gated!(module_batch_atomicity, store, {
    suite::module_batch_atomicity(&store).await;
});
redis_gated!(module_kv_round_trip, store, {
    suite::module_kv_round_trip(&store).await;
});
redis_gated!(module_stream_ordering, store, {
    suite::module_stream_ordering(&store).await;
});
redis_gated!(module_empty_namespace_list, store, {
    suite::module_empty_namespace_list(&store).await;
});

// ---------------------------------------------------------------------
// Redis-only behavior
// ---------------------------------------------------------------------

redis_gated!(index_bootstrap_and_search, store, {
    // ensure_index already ran in connect(); running it again must be a
    // clean no-op ("Index already exists" is swallowed).
    store.ensure_index().await;

    // Write a distinctively named object and find it through FT.SEARCH.
    let mut obj = mush_core::Obj::new(
        mush_core::ObjId(7),
        mush_core::ObjKind::Thing,
        "Xyzzyplugh",
        mush_core::GOD,
        100,
    );
    obj.location = Some(mush_core::LIMBO);
    store
        .write(WriteBatch {
            serial: 1,
            upserts: vec![obj],
            next_id: Some(8),
            ..WriteBatch::default()
        })
        .await
        .expect("write searchable object");

    // Indexing is near-synchronous for JSON docs, but give it a moment.
    let mut found = Vec::new();
    for _ in 0..20 {
        found = store.search_names("Xyzzyplugh").await.expect("ft_search");
        if !found.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        found,
        vec![mush_core::ObjId(7)],
        "FT index must resolve the object by name"
    );
});

redis_gated!(search_special_characters_are_literal, store, {
    // A query full of RediSearch syntax must be escaped into a literal
    // term: no syntax error, just no matches.
    let found = store
        .search_names("no)such|name* -neg @field:(x)")
        .await
        .expect("escaped query must not be a syntax error");
    assert!(found.is_empty());
});

redis_gated!(link_character_rejects_non_array_characters, store, {
    use fred::interfaces::RedisJsonInterface;
    use mush_store::{AccountRec, StoreError};

    store
        .create_account(AccountRec {
            name: "Mangled".into(),
            argon2_hash: "h".into(),
            created: 1,
            characters: vec![],
        })
        .await
        .expect("create account");
    // Clobber the characters field with a non-array. JSON.ARRAPPEND then
    // replies [nil] for the matched path — which must surface as Corrupt,
    // not silently link nothing.
    let _: Option<String> = store
        .client()
        .json_set("account:mangled", "$.characters", "\"oops\"", None)
        .await
        .expect("clobber characters");
    let err = store
        .link_character("mangled", mush_core::ObjId(7), "Ghost")
        .await
        .expect_err("linking into a non-array must fail");
    assert!(
        matches!(err, StoreError::Corrupt(_)),
        "expected Corrupt, got {err:?}"
    );
});

redis_gated!(ban_native_ttl, store, {
    // Bans expire server-side via EXAT — no client filtering. Two bans, two
    // claims: a long-TTL ban proves visibility (immune to client/server clock
    // skew and WAN latency, both real when REDIS_URL crosses an SSH tunnel),
    // and a short-TTL ban proves per-key server-side expiry, polled with a
    // deadline rather than a fixed sleep for the same reason.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    store
        .set_ban("10.0.0.1", Some(now + 60), "long enough to observe")
        .await
        .expect("set long ban");
    store
        .set_ban("10.0.0.2", Some(now + 2), "blink and it's gone")
        .await
        .expect("set short ban");
    let bans = store.get_bans().await.expect("get_bans");
    assert!(
        bans.iter().any(|b| b.ip == "10.0.0.1"),
        "long ban visible before expiry: {bans:?}"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let bans = store.get_bans().await.expect("get_bans while polling");
        if !bans.iter().any(|b| b.ip == "10.0.0.2") {
            assert!(
                bans.iter().any(|b| b.ip == "10.0.0.1"),
                "expiry must be per-key: long ban vanished with the short one"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Redis TTL never expired the short ban: {bans:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
});

redis_gated!(kill9_equivalent_reconnect_load, store, {
    // The store-side half of the M2 exit criterion: write a world, drop the
    // client (a kill -9'd server never says goodbye), reconnect fresh, and
    // load an identical world including the last acked batch.
    suite::empty_seed_load_round_trip(&store).await;
    store
        .write(WriteBatch {
            serial: 99,
            upserts: vec![{
                let mut o = mush_core::Obj::new(
                    mush_core::ObjId(2),
                    mush_core::ObjKind::Thing,
                    "last acked command",
                    mush_core::GOD,
                    100,
                );
                o.location = Some(mush_core::LIMBO);
                o
            }],
            next_id: Some(3),
            durable: true,
            ..WriteBatch::default()
        })
        .await
        .expect("last write");
    drop(store); // no quit(), no drain(): simulated hard kill

    let url = std::env::var("REDIS_URL").expect("checked earlier");
    let store = RedisStore::connect(&url).await.expect("reconnect");
    let snap = store
        .load_world()
        .await
        .expect("load after hard kill")
        .expect("world survived");
    assert_eq!(snap.next_id, 3);
    assert!(
        snap.objects.iter().any(|o| o.name == "last acked command"),
        "the last acked batch must be present after restart"
    );
    store.wipe_namespaces().await.expect("cleanup");
    store.close().await.expect("close");
});
