//! [`RedisStore`] — the authoritative [`ObjectStore`] over Redis 8 via
//! `fred` 10 (RESP3, auto-reconnect, RedisJSON, RediSearch).
//!
//! ## Key layout (PLAN.md "Redis data layout")
//!
//! | key                   | type        | contents                                  |
//! |-----------------------|-------------|-------------------------------------------|
//! | `obj:{id}`            | JSON doc    | one serialized [`mush_core::Obj`]         |
//! | `meta:next_id`        | string int  | id-allocator resume point                 |
//! | `meta:schema_version` | string int  | app-level schema version                  |
//! | `account:{lowername}` | JSON doc    | one [`AccountRec`] (display case inside)  |
//! | `ban:{ip}`            | string JSON | one [`BanRec`], native `EXAT` TTL         |
//! | `token:{hash}`        | string JSON | one [`TokenRecord`], native `EXAT` TTL    |
//! | `mod:{ns}:{key}`      | JSON/stream | module state ([`WriteBatch::module_ops`]) |
//! | `auth:intent:{token}` | string JSON | immutable full authoritative bundle       |
//! | `auth:receipt:{token}`| string JSON | immutable finalized exact witness         |
//! | `auth:idempotency:*`  | string JSON | scoped request-to-bundle binding           |
//! | `auth:pending`        | set         | recovery accelerator (not sole discovery) |
//! | `idx:objs`            | FT index    | see [`RedisStore::ensure_index`]          |
//!
//! Module ops ride the same `MULTI`/`EXEC` as the object writes:
//! `JsonSet` → `JSON.SET mod:{ns}:{key} $ <value>`, `Del` → `DEL`,
//! `StreamAppend` → `XADD mod:{ns}:{key} MAXLEN ~ {maxlen} * data <json>`
//! (approximate trimming: Redis keeps *at least* `maxlen` entries and trims
//! lazily by macro-node, so short streams may exceed the cap — callers read
//! back through [`ObjectStore::stream_range`], which bounds what they see).
//!
//! ## Write-through durability
//!
//! [`write`](ObjectStore::write) issues **one `MULTI`/`EXEC` transaction per
//! batch** — `JSON.SET` per upsert, `DEL` per delete, then the module ops in
//! queue order, `SET meta:next_id` last — and returns when `EXEC` succeeds. That reply *is* the ack: Redis
//! has applied the batch atomically. **Disk** durability is the server's
//! AOF policy (`appendfsync everysec` recommended, `always` for zero-loss),
//! configured server-side — there are no replicas, so `WAIT` adds nothing
//! and is not sent. `durable` batches are trace-tagged but acknowledged
//! identically.
//!
//! Atomicity caveat (Redis semantics, not ours): `MULTI`/`EXEC` aborts the
//! whole transaction on **queue-time** errors, but does **not** roll back a
//! command that fails at execution time (e.g. a `WRONGTYPE` after an
//! external client clobbered an `obj:*` key). Such a failure surfaces as an
//! `Err` from `write` with the batch possibly partially applied. Under this
//! crate's own key discipline that cannot happen; it takes outside
//! interference with the keyspace.
//!
//! Authoritative actions do not use that legacy transaction. A checked Lua
//! reservation stores the complete immutable intent, idempotency binding,
//! pending membership, retention counters, and object-allocation admission
//! before any post-image can materialize. A second checked Lua barrier
//! replays exact post-images, reads every target back without interleaving,
//! publishes the finalized witness, and removes pending membership. Redis
//! script errors are not rollback points, so an execution-time `WRONGTYPE`
//! may leave a prefix of post-images applied; the pre-existing full intent
//! deliberately remains discoverable and exactly replayable.
//!
//! ## Reconnects and outages
//!
//! The client is built with an exponential [`ReconnectPolicy`] (100ms → 30s,
//! unlimited attempts) **and a per-command timeout**
//! ([`COMMAND_TIMEOUT`]): fred does *not* fail commands while the
//! connection is down — it buffers them for replay after a reconnect — so
//! without the timeout an outage would hang callers forever instead of
//! erroring. With it, a command issued during an outage errors within
//! [`COMMAND_TIMEOUT`], which is what lets the world task's policy (pause
//! mutating commands, alert staff, reads continue from RAM) engage.
//! An unresponsive-connection watchdog ([`UNRESPONSIVE_TIMEOUT`]) also
//! force-closes black-holed TCP connections so the reconnect policy takes
//! over. Retrying a timed-out batch is safe even if it later lands anyway:
//! upserts are full-object snapshots and `SET meta:next_id` is absolute, so
//! a duplicate application is idempotent.
//!
//! ## Search index
//!
//! `idx:objs` (`FT.CREATE ... ON JSON PREFIX 1 obj:`) exists for staff
//! tooling and future features; the in-game `@find` walks RAM. Index
//! bootstrap is idempotent ("Index already exists" is success) and
//! **non-fatal**: a Redis build without the search module still boots the
//! game, and [`RedisStore::search_names`] simply errors.

use futures::TryStreamExt;
use futures::future::BoxFuture;
use mush_core::{
    AuthoritativeBundle, CanonicalHash, CommitToken, Obj, ObjId, ReplayWrite, ScopedIdempotencyKey,
};
use serde::{Deserialize, Serialize};

use fred::clients::Client;
use fred::error::Error as FredError;
use fred::interfaces::{
    ClientLike, KeysInterface, LuaInterface, RediSearchInterface, RedisJsonInterface,
    ServerInterface, SetsInterface, StreamsInterface, TransactionInterface,
};
use fred::prelude::{Builder, Config, Expiration, ReconnectPolicy, SetOptions};
use fred::types::redisearch::{
    FtCreateOptions, FtSearchOptions, IndexKind, SearchSchema, SearchSchemaKind,
};
use fred::types::{Key, RespVersion, Value};

use crate::store::{AuthoritativeStore, ObjectStore};
use crate::types::{
    AccountRec, AuthoritativeIntentPage, AuthoritativeReceipt, AuthoritativeScanCursor, BanRec,
    BundleStatus, CharacterRef, IdempotencyStatus, IntentReceipt, MAX_AUTHORITATIVE_DISCOVERY_PAGE,
    MAX_RETAINED_AUTHORITATIVE_BYTES, MAX_RETAINED_AUTHORITATIVE_INTENTS, ModuleWrite,
    PendingAuthoritativeIntent, SshKeyRec, StoreError, TokenRecord, WorldSnapshot, WriteBatch,
    WriteReceipt, validate_module_key, validate_module_ns, validate_module_op, validate_token_hash,
};

/// Prefix of per-object JSON docs.
pub const OBJ_PREFIX: &str = "obj:";
/// Prefix of account JSON docs.
pub const ACCOUNT_PREFIX: &str = "account:";
/// Prefix of siteban keys.
pub const BAN_PREFIX: &str = "ban:";
/// Prefix of web-session token keys (`token:{sha256-hex}`).
pub const TOKEN_PREFIX: &str = "token:";
/// Prefix of module-state keys (`mod:{ns}:{key}`).
pub const MOD_PREFIX: &str = "mod:";
/// Id-allocator resume point.
pub const META_NEXT_ID: &str = "meta:next_id";
/// App-level schema version.
pub const META_SCHEMA_VERSION: &str = "meta:schema_version";
/// Name of the object search index.
pub const OBJ_INDEX: &str = "idx:objs";
/// Prefix of immutable full authoritative intents.
pub const AUTHORITATIVE_INTENT_PREFIX: &str = "auth:intent:";
/// Prefix of immutable finalized authoritative witnesses.
pub const AUTHORITATIVE_RECEIPT_PREFIX: &str = "auth:receipt:";
/// Prefix of scoped idempotency bindings addressed by canonical scope hash.
pub const AUTHORITATIVE_IDEMPOTENCY_PREFIX: &str = "auth:idempotency:";
/// Recovery accelerator; exhaustive intent scanning remains mandatory.
pub const AUTHORITATIVE_PENDING_INDEX: &str = "auth:pending";
/// Count of full intents retained until checkpoint compaction.
pub const AUTHORITATIVE_RETAINED_COUNT: &str = "auth:retained_count";
/// Encoded full-intent bytes retained until checkpoint compaction.
pub const AUTHORITATIVE_RETAINED_BYTES: &str = "auth:retained_bytes";

const RESERVE_AUTHORITATIVE_LUA: &str = include_str!("lua/reserve_authoritative.lua");
const FINALIZE_AUTHORITATIVE_LUA: &str = include_str!("lua/finalize_authoritative.lua");

/// SCAN page size used for bulk key walks.
const SCAN_PAGE: u32 = 500;

/// Per-command timeout. Commands issued while the connection is down error
/// after this long instead of buffering until a reconnect (see the module
/// docs — the whole pause-on-outage policy depends on writes *failing*).
pub const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a connection may sit with an unanswered frame before fred
/// force-closes it and reconnects (catches black-holed TCP).
pub const UNRESPONSIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn obj_key(id: ObjId) -> String {
    format!("{OBJ_PREFIX}{}", id.0)
}

fn account_key(name: &str) -> String {
    format!("{ACCOUNT_PREFIX}{}", name.to_lowercase())
}

fn ban_key(ip: &str) -> String {
    format!("{BAN_PREFIX}{ip}")
}

fn token_key(hash: &str) -> String {
    format!("{TOKEN_PREFIX}{hash}")
}

/// `mod:{ns}:{key}`. Callers validate `ns`/`key` first (see
/// [`crate::types::validate_module_ns`]): with a `:`-free namespace the key
/// is unambiguous.
fn module_key(ns: &str, key: &str) -> String {
    format!("{MOD_PREFIX}{ns}:{key}")
}

fn authoritative_intent_key(token: CommitToken) -> String {
    format!("{AUTHORITATIVE_INTENT_PREFIX}{token}")
}

fn authoritative_receipt_key(token: CommitToken) -> String {
    format!("{AUTHORITATIVE_RECEIPT_PREFIX}{token}")
}

fn authoritative_idempotency_key(key: &ScopedIdempotencyKey) -> Result<String, StoreError> {
    let value = serde_json::to_value(key)?;
    let digest = CanonicalHash::for_canonical_value(
        "authoritative_idempotency_address.v1",
        1,
        &value,
        64 * 1024,
    )
    .map_err(|error| StoreError::Invalid(format!("invalid idempotency identity: {error}")))?;
    Ok(format!("{AUTHORITATIVE_IDEMPOTENCY_PREFIX}{digest}"))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RedisIdempotencyBinding {
    key: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    token: CommitToken,
    bundle_hash: CanonicalHash,
}

impl RedisIdempotencyBinding {
    fn for_bundle(bundle: &AuthoritativeBundle) -> Self {
        Self {
            key: bundle.idempotency().clone(),
            ingress_hash: bundle.ingress_hash(),
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        }
    }
}

struct RedisReplayWrite {
    key: String,
    kind: &'static str,
    post_image: String,
}

fn plan_authoritative_replay(
    bundle: &AuthoritativeBundle,
) -> Result<Vec<RedisReplayWrite>, StoreError> {
    bundle
        .writes()
        .iter()
        .map(|write| {
            Ok(match write {
                ReplayWrite::ObjectSet { object } => RedisReplayWrite {
                    key: obj_key(object.id),
                    kind: "json_set",
                    post_image: serde_json::to_string(object)?,
                },
                ReplayWrite::ObjectDelete { id } => RedisReplayWrite {
                    key: obj_key(*id),
                    kind: "delete",
                    post_image: String::new(),
                },
                ReplayWrite::ObjectAllocatorSet { next_id } => RedisReplayWrite {
                    key: META_NEXT_ID.to_owned(),
                    kind: "string_set",
                    post_image: next_id.to_string(),
                },
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => RedisReplayWrite {
                    key: module_key(namespace.as_str(), key.as_str()),
                    kind: "json_set",
                    post_image: serde_json::to_string(value)?,
                },
                ReplayWrite::ModuleDelete { namespace, key } => RedisReplayWrite {
                    key: module_key(namespace.as_str(), key.as_str()),
                    kind: "delete",
                    post_image: String::new(),
                },
            })
        })
        .collect()
}

fn authoritative_script_error(code: &str) -> StoreError {
    if code == "TOKEN_CONFLICT" {
        return StoreError::AuthoritativeTokenConflict;
    }
    if code == "IDEMPOTENCY_CONFLICT" {
        return StoreError::AuthoritativeIdempotencyConflict;
    }
    if code == "MISSING_INTENT" {
        return StoreError::AuthoritativeIntentMissing;
    }
    if let Some(capacity) = code.strip_prefix("CAPACITY:") {
        let mut parts = capacity.split(':');
        if let (Some(count), Some(bytes), None) = (parts.next(), parts.next(), parts.next())
            && let (Ok(retained_count), Ok(retained_bytes)) =
                (count.parse::<usize>(), bytes.parse::<usize>())
        {
            return StoreError::AuthoritativeCapacity {
                retained_count,
                retained_bytes,
                max_count: MAX_RETAINED_AUTHORITATIVE_INTENTS,
                max_bytes: MAX_RETAINED_AUTHORITATIVE_BYTES,
            };
        }
    }
    if matches!(code, "INVALID_ALLOCATOR" | "MISSING_ALLOCATOR" | "ID_REUSE") {
        return StoreError::Invalid(format!(
            "Redis refused authoritative object allocation: {code}"
        ));
    }
    StoreError::AuthoritativeCorrupt(format!("Redis authoritative script returned {code:?}"))
}

/// The `SCAN` pattern for one module namespace. Two safety properties:
/// the trailing `:` separator keeps `mod:a:*` from matching `mod:ab:...`,
/// and glob metacharacters in `ns` are escaped so a namespace named `a*`
/// scans literally instead of widening the match.
fn module_scan_pattern(ns: &str) -> String {
    let mut escaped = String::with_capacity(ns.len() * 2);
    for c in ns.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!("{MOD_PREFIX}{escaped}:*")
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Whether a Redis error means "that key holds a different type". Core
/// Redis says `WRONGTYPE ...`; RedisJSON (live Redis 8 verified) says
/// `Existing key has wrong Redis type` — match both phrasings.
fn is_wrong_type_err(e: &fred::error::Error) -> bool {
    let msg = e.to_string().to_ascii_lowercase();
    msg.contains("wrongtype") || msg.contains("wrong redis type")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonJsonModuleKey {
    Stream,
    Gone,
}

/// Classify a module key after `JSON.GET` has already reported wrong type.
/// Only a positively identified stream may be omitted from the JSON listing;
/// every other live Redis type is authoritative corruption.
fn classify_non_json_module_key(
    full_key: &str,
    redis_type: &str,
) -> Result<NonJsonModuleKey, StoreError> {
    match redis_type {
        "stream" => Ok(NonJsonModuleKey::Stream),
        "none" => Ok(NonJsonModuleKey::Gone),
        unexpected => Err(StoreError::Corrupt(format!(
            "module key {full_key:?} has unexpected Redis type {unexpected:?}"
        ))),
    }
}

/// `JSON.GET key $` → the raw doc, unwrapped from RedisJSON's one-element
/// path-match array. `Ok(None)` when the key is gone.
async fn json_doc(client: &Client, key: &str) -> Result<Option<serde_json::Value>, StoreError> {
    let raw: Option<String> = client
        .json_get::<Option<String>, _, &str, &str, &str, _>(key, None, None, None, "$")
        .await?;
    let Some(raw) = raw else { return Ok(None) };
    let mut matches: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
    if matches.is_empty() {
        return Ok(None);
    }
    Ok(Some(matches.swap_remove(0)))
}

/// See the [module docs](self).
pub struct RedisStore {
    client: Client,
}

impl RedisStore {
    /// Connect to `url` (`redis://[user:pass@]host:port[/db]`), RESP3, with
    /// auto-reconnect and per-command timeouts, then bootstrap the search
    /// index (non-fatal).
    ///
    /// Fails fast if the server is unreachable — per PLAN, boot dies with a
    /// clear error rather than running without persistence.
    pub async fn connect(url: &str) -> Result<RedisStore, StoreError> {
        let store = RedisStore::connect_read_only(url).await?;
        store.ensure_index().await;
        Ok(store)
    }

    /// Connect without running any write-capable bootstrap.
    ///
    /// This narrow constructor exists for quarantine recovery: callers may
    /// issue authoritative reads before deciding whether ordinary writable
    /// boot is safe. In particular it does **not** call [`Self::ensure_index`]
    /// or run migrations. The returned type still implements [`ObjectStore`],
    /// so recovery code must expose it only through a read-only interface.
    pub async fn connect_read_only(url: &str) -> Result<RedisStore, StoreError> {
        let builder = RedisStore::configured_builder(url)?;
        let client = builder.build()?;
        client.init().await?;
        Ok(RedisStore { client })
    }

    /// The client [`Builder`] [`connect`](RedisStore::connect) uses: RESP3,
    /// fail-fast boot, unlimited exponential reconnects, and — critically —
    /// [`COMMAND_TIMEOUT`] / [`UNRESPONSIVE_TIMEOUT`] so a Redis outage
    /// surfaces as command errors instead of indefinite buffering (fred's
    /// default is to buffer commands while disconnected; see module docs).
    /// Factored out so the configuration is unit-testable without a server.
    pub fn configured_builder(url: &str) -> Result<Builder, StoreError> {
        let mut config = Config::from_url(url)?;
        config.version = RespVersion::RESP3;
        config.fail_fast = true;
        let mut builder = Builder::from_config(config);
        builder
            .set_policy(ReconnectPolicy::new_exponential(0, 100, 30_000, 2))
            .with_performance_config(|p| {
                p.default_command_timeout = COMMAND_TIMEOUT;
            })
            .with_connection_config(|c| {
                c.unresponsive.max_timeout = Some(UNRESPONSIVE_TIMEOUT);
            });
        Ok(builder)
    }

    /// Wrap an already-connected client (tests, pooling experiments).
    #[must_use]
    pub fn from_client(client: Client) -> RedisStore {
        RedisStore { client }
    }

    /// The underlying `fred` client, for subsystems that need raw commands
    /// (streams, tokens) until they grow their own store methods.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.client
    }

    async fn load_validated_authoritative(
        &self,
        token: CommitToken,
    ) -> Result<Option<(AuthoritativeBundle, Option<AuthoritativeReceipt>)>, StoreError> {
        let intent_key = authoritative_intent_key(token);
        let intent_doc: Option<String> = self.client.get(intent_key).await?;
        let Some(intent_doc) = intent_doc else {
            let receipt: Option<String> = self.client.get(authoritative_receipt_key(token)).await?;
            if receipt.is_some() {
                return Err(StoreError::AuthoritativeCorrupt(
                    "finalized Redis receipt has no retained intent".to_owned(),
                ));
            }
            return Ok(None);
        };
        let bundle: AuthoritativeBundle = serde_json::from_str(&intent_doc)
            .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
        if bundle.token() != token {
            return Err(StoreError::AuthoritativeCorrupt(
                "Redis intent is stored under the wrong token".to_owned(),
            ));
        }
        let binding_doc: Option<String> = self
            .client
            .get(authoritative_idempotency_key(bundle.idempotency())?)
            .await?;
        let binding_doc = binding_doc.ok_or_else(|| {
            StoreError::AuthoritativeCorrupt(
                "Redis intent is missing its idempotency binding".to_owned(),
            )
        })?;
        let binding: RedisIdempotencyBinding = serde_json::from_str(&binding_doc)
            .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
        if binding != RedisIdempotencyBinding::for_bundle(&bundle) {
            return Err(StoreError::AuthoritativeCorrupt(
                "Redis intent disagrees with its idempotency binding".to_owned(),
            ));
        }
        let receipt_doc: Option<String> = self.client.get(authoritative_receipt_key(token)).await?;
        let receipt = receipt_doc
            .map(|doc| {
                let receipt: AuthoritativeReceipt = serde_json::from_str(&doc)
                    .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
                if receipt.bundle() != &bundle {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "Redis receipt disagrees with its retained intent".to_owned(),
                    ));
                }
                Ok(receipt)
            })
            .transpose()?;
        Ok(Some((bundle, receipt)))
    }

    /// Idempotently create `idx:objs`:
    ///
    /// ```text
    /// FT.CREATE idx:objs ON JSON PREFIX 1 obj: SCHEMA
    ///   $.name  AS name  TEXT
    ///   $.kind  AS kind  TAG
    ///   $.owner AS owner NUMERIC
    ///   $.zone  AS zone  NUMERIC
    /// ```
    ///
    /// "Index already exists" is success; any other failure (e.g. no search
    /// module) is logged and swallowed — the index powers staff tooling,
    /// not gameplay.
    pub async fn ensure_index(&self) {
        let options = FtCreateOptions {
            on: Some(IndexKind::JSON),
            prefixes: vec![OBJ_PREFIX.into()],
            ..Default::default()
        };
        let schema = vec![
            SearchSchema {
                field_name: "$.name".into(),
                alias: Some("name".into()),
                kind: SearchSchemaKind::Text {
                    sortable: false,
                    unf: false,
                    nostem: false,
                    phonetic: None,
                    weight: None,
                    withsuffixtrie: false,
                    noindex: false,
                },
            },
            SearchSchema {
                field_name: "$.kind".into(),
                alias: Some("kind".into()),
                kind: SearchSchemaKind::Tag {
                    sortable: false,
                    unf: false,
                    separator: None,
                    casesensitive: false,
                    withsuffixtrie: false,
                    noindex: false,
                },
            },
            SearchSchema {
                field_name: "$.owner".into(),
                alias: Some("owner".into()),
                kind: SearchSchemaKind::Numeric {
                    sortable: false,
                    unf: false,
                    noindex: false,
                },
            },
            SearchSchema {
                field_name: "$.zone".into(),
                alias: Some("zone".into()),
                kind: SearchSchemaKind::Numeric {
                    sortable: false,
                    unf: false,
                    noindex: false,
                },
            },
        ];
        match self
            .client
            .ft_create::<(), _>(OBJ_INDEX, options, schema)
            .await
        {
            Ok(()) => tracing::info!(index = OBJ_INDEX, "search index created"),
            Err(e)
                if e.to_string()
                    .to_ascii_lowercase()
                    .contains("already exists") =>
            {
                tracing::debug!(index = OBJ_INDEX, "search index already exists");
            }
            Err(e) => {
                tracing::warn!(index = OBJ_INDEX, error = %e,
                    "search index unavailable (continuing without it)");
            }
        }
    }

    /// `FT.SEARCH idx:objs @name:(query)` → matching object ids. Proves the
    /// index works; staff tooling grows from here (in-game `@find` walks
    /// RAM and never calls this). The query is treated as a **literal
    /// term**: RediSearch syntax characters are escaped, so a name like
    /// `oddball)` or `a|b` searches for itself instead of erroring.
    pub async fn search_names(&self, name_query: &str) -> Result<Vec<ObjId>, StoreError> {
        let options = FtSearchOptions {
            nocontent: true,
            ..Default::default()
        };
        let escaped = escape_search_term(name_query);
        let raw: Value = self
            .client
            .ft_search(OBJ_INDEX, format!("@name:({escaped})"), options)
            .await?;
        // RESP2 replies [count, key, key, ...]; RESP3 replies a map with a
        // `results` array of per-doc maps. Rather than chase the shape, walk
        // the whole reply for `obj:{id}` strings.
        let json = raw.into_json()?;
        let mut ids = Vec::new();
        collect_obj_ids(&json, &mut ids);
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// Collect every key in the connected database matching `pattern`,
    /// deduplicated: SCAN may legally return a key more than once (keyspace
    /// rehash mid-scan), and a duplicated `obj:*` key would otherwise fail
    /// a boot-time load as a `DuplicateId`.
    async fn scan_keys(&self, pattern: &str) -> Result<Vec<String>, StoreError> {
        let keys: Vec<Key> = self
            .client
            .scan_buffered(pattern, Some(SCAN_PAGE), None)
            .try_collect()
            .await?;
        let unique: std::collections::BTreeSet<String> = keys
            .iter()
            .filter_map(|k| k.as_str().map(ToOwned::to_owned))
            .collect();
        Ok(unique.into_iter().collect())
    }

    /// Test/ops helper: delete every key in this crate's namespaces
    /// (`obj:*`, `account:*`, `ban:*`, `token:*`, `meta:*`, `mod:*`) in the
    /// connected database. Deliberately not `FLUSHDB` — it only touches
    /// what this crate owns.
    #[doc(hidden)]
    pub async fn wipe_namespaces(&self) -> Result<(), StoreError> {
        for pattern in [
            "obj:*",
            "account:*",
            "ban:*",
            "token:*",
            "meta:*",
            "mod:*",
            "auth:*",
        ] {
            let keys = self.scan_keys(pattern).await?;
            if !keys.is_empty() {
                let _: i64 = self.client.del(keys).await?;
            }
        }
        Ok(())
    }
}

/// Escape a string so RediSearch treats it as a literal term: every
/// character outside `[A-Za-z0-9_]` is prefixed with `\` (the documented
/// RediSearch escape), neutralizing query syntax like `|`, `*`, `-`, `(`,
/// `)`, `@`, and separating punctuation.
fn escape_search_term(term: &str) -> String {
    let mut out = String::with_capacity(term.len() * 2);
    for c in term.chars() {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Recursively collect `ObjId`s from any `"obj:{id}"` string in a search
/// reply.
fn collect_obj_ids(value: &serde_json::Value, out: &mut Vec<ObjId>) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(rest) = s.strip_prefix(OBJ_PREFIX)
                && let Ok(n) = rest.parse::<u32>()
            {
                out.push(ObjId(n));
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_obj_ids(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_obj_ids(item, out);
            }
        }
        _ => {}
    }
}

impl AuthoritativeStore for RedisStore {
    fn reserve_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        Box::pin(async move {
            let token = bundle.token();
            let bundle_hash = bundle.bundle_hash();
            let intent_doc = serde_json::to_string(&bundle)?;
            let intent_bytes = intent_doc.len();
            let binding = RedisIdempotencyBinding::for_bundle(&bundle);
            let binding_doc = serde_json::to_string(&binding)?;
            let proposed_next_id = bundle.writes().iter().find_map(|write| match write {
                ReplayWrite::ObjectAllocatorSet { next_id } => Some(*next_id),
                _ => None,
            });
            let object_sets: Vec<_> = bundle
                .writes()
                .iter()
                .filter_map(|write| match write {
                    ReplayWrite::ObjectSet { object } => Some(object.id),
                    _ => None,
                })
                .collect();
            let mut keys = vec![
                authoritative_intent_key(token),
                authoritative_receipt_key(token),
                authoritative_idempotency_key(bundle.idempotency())?,
                AUTHORITATIVE_PENDING_INDEX.to_owned(),
                AUTHORITATIVE_RETAINED_COUNT.to_owned(),
                AUTHORITATIVE_RETAINED_BYTES.to_owned(),
                META_NEXT_ID.to_owned(),
            ];
            keys.extend(object_sets.iter().map(|id| obj_key(*id)));
            let mut args = vec![
                intent_doc,
                binding_doc,
                token.to_string(),
                intent_bytes.to_string(),
                MAX_RETAINED_AUTHORITATIVE_INTENTS.to_string(),
                MAX_RETAINED_AUTHORITATIVE_BYTES.to_string(),
                proposed_next_id.map_or_else(String::new, |next_id| next_id.to_string()),
                object_sets.len().to_string(),
            ];
            args.extend(object_sets.iter().map(|id| id.0.to_string()));
            let code: String = self
                .client
                .eval(RESERVE_AUTHORITATIVE_LUA, keys, args)
                .await?;
            let resumed = match code.as_str() {
                "NEW" => false,
                "RESUMED" => true,
                _ => return Err(authoritative_script_error(&code)),
            };
            let Some((stored, _)) = self.load_validated_authoritative(token).await? else {
                return Err(StoreError::AuthoritativeCorrupt(
                    "Redis reservation returned without a retained intent".to_owned(),
                ));
            };
            if stored != bundle {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            Ok(IntentReceipt {
                token,
                bundle_hash,
                resumed,
            })
        })
    }

    fn authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
        ingress_hash: CanonicalHash,
    ) -> BoxFuture<'a, Result<IdempotencyStatus, StoreError>> {
        Box::pin(async move {
            let binding_doc: Option<String> =
                self.client.get(authoritative_idempotency_key(key)?).await?;
            let Some(binding_doc) = binding_doc else {
                return Ok(IdempotencyStatus::Vacant);
            };
            let binding: RedisIdempotencyBinding = serde_json::from_str(&binding_doc)
                .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
            if &binding.key != key {
                return Err(StoreError::AuthoritativeCorrupt(
                    "Redis idempotency address contains a different scoped key".to_owned(),
                ));
            }
            let Some((bundle, receipt)) = self.load_validated_authoritative(binding.token).await?
            else {
                return Err(StoreError::AuthoritativeCorrupt(
                    "Redis idempotency binding names no retained intent".to_owned(),
                ));
            };
            if binding != RedisIdempotencyBinding::for_bundle(&bundle) {
                return Err(StoreError::AuthoritativeCorrupt(
                    "Redis idempotency binding disagrees with its retained intent".to_owned(),
                ));
            }
            if ingress_hash != binding.ingress_hash {
                return Ok(IdempotencyStatus::Conflict);
            }
            Ok(receipt.map_or(
                IdempotencyStatus::PendingExact {
                    token: binding.token,
                    bundle_hash: binding.bundle_hash,
                },
                |receipt| IdempotencyStatus::FinalizedExact(Box::new(receipt)),
            ))
        })
    }

    fn reconcile_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeReceipt, StoreError>> {
        Box::pin(async move {
            let Some((bundle, prior_receipt)) = self.load_validated_authoritative(token).await?
            else {
                return Err(StoreError::AuthoritativeIntentMissing);
            };
            if bundle.bundle_hash() != expected_bundle_hash {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            if let Some(receipt) = prior_receipt {
                let _: i64 = self
                    .client
                    .srem(AUTHORITATIVE_PENDING_INDEX, token.to_string())
                    .await?;
                return Ok(receipt);
            }
            let intent_doc = serde_json::to_string(&bundle)?;
            let receipt = AuthoritativeReceipt::new(bundle.clone());
            let receipt_doc = serde_json::to_string(&receipt)?;
            let binding_doc = serde_json::to_string(&RedisIdempotencyBinding::for_bundle(&bundle))?;
            let planned = plan_authoritative_replay(&bundle)?;
            let mut keys = vec![
                authoritative_intent_key(token),
                authoritative_receipt_key(token),
                authoritative_idempotency_key(bundle.idempotency())?,
                AUTHORITATIVE_PENDING_INDEX.to_owned(),
            ];
            keys.extend(planned.iter().map(|write| write.key.clone()));
            let mut args = vec![
                intent_doc,
                receipt_doc,
                binding_doc,
                token.to_string(),
                planned.len().to_string(),
            ];
            for write in &planned {
                args.push(write.kind.to_owned());
                args.push(write.post_image.clone());
            }
            let code: String = self
                .client
                .eval(FINALIZE_AUTHORITATIVE_LUA, keys, args)
                .await?;
            if code != "FINALIZED" {
                return Err(authoritative_script_error(&code));
            }
            let Some((stored, stored_receipt)) = self.load_validated_authoritative(token).await?
            else {
                return Err(StoreError::AuthoritativeCorrupt(
                    "Redis finalization returned without a retained intent".to_owned(),
                ));
            };
            if stored.bundle_hash() != expected_bundle_hash {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            stored_receipt.ok_or_else(|| {
                StoreError::AuthoritativeCorrupt(
                    "Redis finalization returned without a finalized witness".to_owned(),
                )
            })
        })
    }

    fn authoritative_status(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<BundleStatus, StoreError>> {
        Box::pin(async move {
            let Some((bundle, receipt)) = self.load_validated_authoritative(token).await? else {
                return Ok(BundleStatus::Absent);
            };
            if bundle.bundle_hash() != expected_bundle_hash {
                return Ok(BundleStatus::Conflict {
                    stored_bundle_hash: bundle.bundle_hash(),
                });
            }
            Ok(receipt.map_or(BundleStatus::PendingExact, |receipt| {
                BundleStatus::FinalizedExact(Box::new(receipt))
            }))
        })
    }

    fn pending_authoritative_intents(
        &self,
    ) -> BoxFuture<'_, Result<Vec<PendingAuthoritativeIntent>, StoreError>> {
        Box::pin(async move {
            let mut tokens: Vec<String> = self.client.smembers(AUTHORITATIVE_PENDING_INDEX).await?;
            tokens.sort();
            tokens.dedup();
            if tokens.len() > MAX_RETAINED_AUTHORITATIVE_INTENTS {
                return Err(StoreError::AuthoritativeCorrupt(format!(
                    "Redis pending index contains {} entries; maximum is {}",
                    tokens.len(),
                    MAX_RETAINED_AUTHORITATIVE_INTENTS
                )));
            }
            let mut pending = Vec::with_capacity(tokens.len());
            for token in tokens {
                let token = CommitToken::from_hex(&token).map_err(|error| {
                    StoreError::AuthoritativeCorrupt(format!(
                        "Redis pending index contains invalid token: {error}"
                    ))
                })?;
                let Some((bundle, _)) = self.load_validated_authoritative(token).await? else {
                    return Err(StoreError::AuthoritativeCorrupt(format!(
                        "Redis pending index token {token} has no retained intent"
                    )));
                };
                pending.push(PendingAuthoritativeIntent {
                    token,
                    bundle_hash: bundle.bundle_hash(),
                });
            }
            Ok(pending)
        })
    }

    fn scan_authoritative_intents(
        &self,
        cursor: Option<AuthoritativeScanCursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<AuthoritativeIntentPage, StoreError>> {
        Box::pin(async move {
            if limit == 0 || limit > MAX_AUTHORITATIVE_DISCOVERY_PAGE {
                return Err(StoreError::Invalid(format!(
                    "authoritative discovery page limit must be 1..={MAX_AUTHORITATIVE_DISCOVERY_PAGE}"
                )));
            }
            let cursor = cursor.map_or_else(|| "0".to_owned(), |cursor| cursor.as_str().to_owned());
            let count = u32::try_from(limit).map_err(|_| {
                StoreError::Invalid("authoritative discovery limit is out of range".to_owned())
            })?;
            let (next, keys): (String, Vec<Key>) = self
                .client
                .scan_page(
                    cursor,
                    format!("{AUTHORITATIVE_INTENT_PREFIX}*"),
                    Some(count),
                    None,
                )
                .await?;
            let unique: std::collections::BTreeSet<String> = keys
                .iter()
                .filter_map(|key| key.as_str().map(ToOwned::to_owned))
                .collect();
            if unique.len() > MAX_AUTHORITATIVE_DISCOVERY_PAGE {
                return Err(StoreError::AuthoritativeCorrupt(format!(
                    "Redis returned {} authoritative intents in one scan page; maximum is {}",
                    unique.len(),
                    MAX_AUTHORITATIVE_DISCOVERY_PAGE
                )));
            }
            let mut intents = Vec::with_capacity(unique.len());
            for key in unique {
                let token = key
                    .strip_prefix(AUTHORITATIVE_INTENT_PREFIX)
                    .ok_or_else(|| {
                        StoreError::AuthoritativeCorrupt(
                            "Redis authoritative scan returned a key outside its prefix".to_owned(),
                        )
                    })
                    .and_then(|token| {
                        CommitToken::from_hex(token).map_err(|error| {
                            StoreError::AuthoritativeCorrupt(format!(
                                "Redis authoritative intent key has invalid token: {error}"
                            ))
                        })
                    })?;
                if let Some((bundle, _)) = self.load_validated_authoritative(token).await? {
                    intents.push(bundle);
                }
            }
            let next_cursor = if next.as_str() == "0" {
                None
            } else {
                Some(AuthoritativeScanCursor::new(next.as_str())?)
            };
            Ok(AuthoritativeIntentPage {
                intents,
                next_cursor,
            })
        })
    }
}

impl ObjectStore for RedisStore {
    fn authoritative_store(&self) -> Option<&dyn AuthoritativeStore> {
        Some(self)
    }

    fn bootstrap_writable(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            self.ensure_index().await;
            Ok(())
        })
    }

    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        Box::pin(async move {
            let keys = self.scan_keys("obj:*").await?;
            let next_id: Option<i64> = self.client.get(META_NEXT_ID).await?;
            if keys.is_empty() && next_id.is_none() {
                return Ok(None);
            }
            let Some(next_id) = next_id else {
                return Err(StoreError::Corrupt(
                    "obj:* keys present but meta:next_id missing".into(),
                ));
            };
            let next_id = u32::try_from(next_id).map_err(|_| {
                StoreError::Corrupt(format!("meta:next_id out of range: {next_id}"))
            })?;

            let mut objects = Vec::with_capacity(keys.len());
            let mut corrupt = 0usize;
            for key in &keys {
                match json_doc(&self.client, key).await? {
                    // Key vanished between SCAN and GET: fine, skip.
                    None => {}
                    Some(doc) => match serde_json::from_value::<Obj>(doc) {
                        Ok(obj) => {
                            if obj_key(obj.id) != *key {
                                tracing::warn!(key, id = %obj.id, "object doc id/key mismatch");
                            }
                            objects.push(obj);
                        }
                        Err(e) => {
                            corrupt += 1;
                            tracing::error!(key, error = %e, "skipping corrupt object doc");
                        }
                    },
                }
            }
            if corrupt > 0 {
                tracing::error!(corrupt, "corrupt object docs skipped during load");
            }
            let schema_version: Option<i64> = self.client.get(META_SCHEMA_VERSION).await?;
            let schema_version = match schema_version {
                Some(v) => u32::try_from(v).map_err(|_| {
                    StoreError::Corrupt(format!("meta:schema_version out of range: {v}"))
                })?,
                None => {
                    tracing::warn!("non-empty store without schema_version; assuming v1");
                    1
                }
            };
            Ok(Some(WorldSnapshot {
                objects,
                next_id,
                schema_version,
                corrupt_docs: corrupt,
            }))
        })
    }

    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        Box::pin(async move {
            let serial = batch.serial;
            if batch.is_empty() {
                return Ok(WriteReceipt { serial });
            }
            // Validate + serialize everything up front; only then open the
            // MULTI — a refused op leaves the store untouched.
            let mut docs = Vec::with_capacity(batch.upserts.len());
            for obj in &batch.upserts {
                docs.push((obj_key(obj.id), serde_json::to_string(obj)?));
            }
            enum Planned {
                JsonSet {
                    key: String,
                    doc: String,
                },
                Del {
                    key: String,
                },
                StreamAppend {
                    key: String,
                    doc: String,
                    maxlen: u32,
                },
            }
            let mut planned = Vec::with_capacity(batch.module_ops.len());
            for op in &batch.module_ops {
                validate_module_op(op)?;
                planned.push(match &op.op {
                    ModuleWrite::JsonSet { key, value } => Planned::JsonSet {
                        key: module_key(&op.ns, key),
                        doc: serde_json::to_string(value)?,
                    },
                    ModuleWrite::Del { key } => Planned::Del {
                        key: module_key(&op.ns, key),
                    },
                    ModuleWrite::StreamAppend { key, value, maxlen } => Planned::StreamAppend {
                        key: module_key(&op.ns, key),
                        doc: serde_json::to_string(value)?,
                        maxlen: *maxlen,
                    },
                });
            }
            let trx = self.client.multi();
            for (key, doc) in &docs {
                let _: () = trx.json_set(key.as_str(), "$", doc.as_str(), None).await?;
            }
            for id in &batch.deletes {
                let _: () = trx.del(obj_key(*id)).await?;
            }
            // Module ops after the object writes, in queue order.
            for op in &planned {
                match op {
                    Planned::JsonSet { key, doc } => {
                        let _: () = trx.json_set(key.as_str(), "$", doc.as_str(), None).await?;
                    }
                    Planned::Del { key } => {
                        let _: () = trx.del(key.as_str()).await?;
                    }
                    Planned::StreamAppend { key, doc, maxlen } => {
                        let _: () = trx
                            .xadd(
                                key.as_str(),
                                false,
                                ("MAXLEN", "~", i64::from(*maxlen)),
                                "*",
                                ("data", doc.as_str()),
                            )
                            .await?;
                    }
                }
            }
            if let Some(next_id) = batch.next_id {
                let _: () = trx
                    .set(META_NEXT_ID, i64::from(next_id), None, None, false)
                    .await?;
            }
            // EXEC. Its success is the ack; disk durability is the server's
            // AOF policy (see module docs — no replicas, so no WAIT).
            let _: Value = trx.exec(true).await?;
            if batch.durable {
                tracing::debug!(serial, "durable batch committed");
            }
            Ok(WriteReceipt { serial })
        })
    }

    fn create_account(&self, rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            if rec.name.is_empty() {
                return Err(StoreError::Invalid("empty account name".into()));
            }
            let key = account_key(&rec.name);
            let doc = serde_json::to_string(&rec)?;
            // JSON.SET ... NX: atomic reserve — exactly one concurrent
            // creator sees OK, the rest see nil.
            let outcome: Option<String> = self
                .client
                .json_set(key, "$", doc, Some(SetOptions::NX))
                .await?;
            if outcome.is_none() {
                return Err(StoreError::AccountExists);
            }
            Ok(())
        })
    }

    fn get_account<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>> {
        Box::pin(async move {
            match json_doc(&self.client, &account_key(name)).await? {
                Some(doc) => Ok(Some(serde_json::from_value(doc)?)),
                None => Ok(None),
            }
        })
    }

    fn link_character<'a>(
        &'a self,
        account: &'a str,
        id: ObjId,
        char_name: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let entry = serde_json::to_string(&CharacterRef {
                id,
                name: char_name.to_string(),
            })?;
            // JSON.ARRAPPEND is a single atomic server-side append. A
            // missing key errors; a present key with no matching path
            // returns an empty match set; a path that matches something
            // that is NOT an array returns nil for that path (an array of
            // nulls) — silently linking nothing, so treat it as corrupt too.
            let appended: Value = match self
                .client
                .json_arrappend(account_key(account), "$.characters", vec![entry])
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    let exists: i64 = self.client.exists(account_key(account)).await?;
                    if exists == 0 {
                        return Err(StoreError::NoSuchAccount(account.to_string()));
                    }
                    return Err(e.into());
                }
            };
            if matches!(&appended, Value::Array(a)
                if a.is_empty() || a.iter().all(|v| matches!(v, Value::Null)))
            {
                return Err(StoreError::Corrupt(format!(
                    "account doc for {account:?} has no characters array"
                )));
            }
            Ok(())
        })
    }

    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>> {
        Box::pin(async move {
            let keys = self.scan_keys("account:*").await?;
            let mut out = Vec::with_capacity(keys.len());
            for key in &keys {
                match json_doc(&self.client, key).await? {
                    None => {}
                    Some(doc) => match serde_json::from_value::<AccountRec>(doc) {
                        Ok(rec) => out.push(rec),
                        Err(e) => tracing::error!(key, error = %e, "skipping corrupt account doc"),
                    },
                }
            }
            Ok(out)
        })
    }

    fn set_account_password<'a>(
        &'a self,
        account: &'a str,
        argon2_hash: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let key = account_key(account);
            let value = serde_json::to_string(argon2_hash)?;
            // One atomic server-side field replace. XX = only set an
            // existing path, so a concurrent create/update never turns
            // into a partial doc. When the condition cannot be met the
            // reply is nil (Redis 8 answers nil for a missing key OR a
            // missing path; older builds errored on the missing key) — an
            // existence check then splits "no such account" (false) from a
            // doc missing its argon2_hash field (corrupt, never silently
            // "updated"), like link_character.
            let outcome: Result<Option<String>, _> = self
                .client
                .json_set(key.as_str(), "$.argon2_hash", value, Some(SetOptions::XX))
                .await;
            let unmet = match outcome {
                Ok(Some(_)) => return Ok(true),
                Ok(None) => Ok(()),
                Err(e) => Err(e),
            };
            let exists: i64 = self.client.exists(key.as_str()).await?;
            if exists == 0 {
                return Ok(false);
            }
            match unmet {
                Ok(()) => Err(StoreError::Corrupt(format!(
                    "account doc for {account:?} has no argon2_hash field"
                ))),
                Err(e) => Err(e.into()),
            }
        })
    }

    fn set_account_ssh_keys<'a>(
        &'a self,
        account: &'a str,
        keys: &'a [SshKeyRec],
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let key = account_key(account);
            // Unlike set_account_password, XX is wrong here: account docs
            // created before ssh_keys existed have no $.ssh_keys path, so an
            // XX set (path-must-exist) would fail on exactly the back-compat
            // case this method must serve. Instead confirm the account key
            // exists, then set the whole array WITHOUT XX — Redis creates the
            // path when its parent (the doc root) is present. serde_json emits
            // `[]` for an empty slice, clearing the keys.
            let exists: i64 = self.client.exists(key.as_str()).await?;
            if exists == 0 {
                return Ok(false);
            }
            let value = serde_json::to_string(keys)?;
            let _: Option<String> = self
                .client
                .json_set(key.as_str(), "$.ssh_keys", value, None)
                .await?;
            Ok(true)
        })
    }

    fn set_ban<'a>(
        &'a self,
        ip: &'a str,
        until_unix: Option<i64>,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            if ip.is_empty() {
                return Err(StoreError::Invalid("empty ban ip".into()));
            }
            let key = ban_key(ip);
            if let Some(until) = until_unix
                && until <= now_unix()
            {
                // Already lapsed: equivalent to the key expiring instantly.
                let _: i64 = self.client.del(key).await?;
                return Ok(());
            }
            let rec = BanRec {
                ip: ip.to_string(),
                until_unix,
                reason: reason.to_string(),
            };
            let doc = serde_json::to_string(&rec)?;
            let expire = until_unix.map(Expiration::EXAT);
            let _: () = self.client.set(key, doc, expire, None, false).await?;
            Ok(())
        })
    }

    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>> {
        Box::pin(async move {
            let keys = self.scan_keys("ban:*").await?;
            let mut out = Vec::with_capacity(keys.len());
            for key in &keys {
                // TTL may have fired between SCAN and GET; None is fine.
                let doc: Option<String> = self.client.get(key.as_str()).await?;
                let Some(doc) = doc else { continue };
                match serde_json::from_str::<BanRec>(&doc) {
                    Ok(rec) => out.push(rec),
                    Err(e) => tracing::error!(key, error = %e, "skipping corrupt ban doc"),
                }
            }
            Ok(out)
        })
    }

    fn clear_ban<'a>(&'a self, ip: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let removed: i64 = self.client.del(ban_key(ip)).await?;
            Ok(removed > 0)
        })
    }

    fn token_put(&self, hash: String, rec: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            validate_token_hash(&hash)?;
            let key = token_key(&hash);
            if rec.expires <= now_unix() {
                // Already lapsed: equivalent to the key expiring instantly.
                let _: i64 = self.client.del(key).await?;
                return Ok(());
            }
            let doc = serde_json::to_string(&rec)?;
            let _: () = self
                .client
                .set(key, doc, Some(Expiration::EXAT(rec.expires)), None, false)
                .await?;
            Ok(())
        })
    }

    fn token_get<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            // Expiry is the server's native TTL (EXAT stamped at put/touch):
            // an expired token is simply absent. No client-side clock check —
            // the server clock is authoritative, exactly as for bans.
            let doc: Option<String> = self.client.get(token_key(hash)).await?;
            match doc {
                Some(doc) => Ok(Some(serde_json::from_str(&doc)?)),
                None => Ok(None),
            }
        })
    }

    fn token_touch<'a>(
        &'a self,
        hash: &'a str,
        new_expires: i64,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let key = token_key(hash);
            // Read-modify-write: the record's `expires` field and the key
            // TTL move together. Two concurrent touches race benignly (last
            // write wins — both extend); a touch racing a delete could
            // rewrite the key for at most one sliding window, which is the
            // same as having touched just before the delete.
            let doc: Option<String> = self.client.get(key.as_str()).await?;
            let Some(doc) = doc else { return Ok(false) };
            if new_expires <= now_unix() {
                // Sliding into the past is a delete.
                let _: i64 = self.client.del(key).await?;
                return Ok(true);
            }
            let mut rec: TokenRecord = serde_json::from_str(&doc)?;
            rec.expires = new_expires;
            let doc = serde_json::to_string(&rec)?;
            let _: () = self
                .client
                .set(key, doc, Some(Expiration::EXAT(new_expires)), None, false)
                .await?;
            Ok(true)
        })
    }

    fn token_del<'a>(&'a self, hash: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let removed: i64 = self.client.del(token_key(hash)).await?;
            Ok(removed > 0)
        })
    }

    fn tokens_del_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<u64, StoreError>> {
        Box::pin(async move {
            let account_lc = account.to_lowercase();
            let keys = self.scan_keys("token:*").await?;
            let mut live = 0u64;
            for key in &keys {
                // TTL may have fired between SCAN and GET; None is fine.
                let doc: Option<String> = self.client.get(key.as_str()).await?;
                let Some(doc) = doc else { continue };
                match serde_json::from_str::<TokenRecord>(&doc) {
                    Ok(rec) if rec.account.to_lowercase() == account_lc => {
                        let removed: i64 = self.client.del(key.as_str()).await?;
                        live += u64::try_from(removed).unwrap_or(0);
                    }
                    Ok(_) => {}
                    Err(e) => tracing::error!(key, error = %e, "skipping corrupt token doc"),
                }
            }
            Ok(live)
        })
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            validate_module_key(key)?;
            json_doc(&self.client, &module_key(ns, key)).await
        })
    }

    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            let prefix = format!("{MOD_PREFIX}{ns}:");
            let keys = self.scan_keys(&module_scan_pattern(ns)).await?;
            let mut out = Vec::with_capacity(keys.len());
            for full_key in &keys {
                // The namespace mixes JSON docs and streams under one
                // prefix. JSON.GET on either a legitimate stream *or* a
                // type-clobbered key is a wrong-type error, so the error by
                // itself is not enough to decide that this key belongs to
                // stream_range. Positively identify the Redis type before
                // skipping it; otherwise corrupt authoritative state could
                // silently disappear during boot.
                let doc = match json_doc(&self.client, full_key).await {
                    Ok(doc) => doc,
                    Err(StoreError::Redis(e)) if is_wrong_type_err(&e) => {
                        let redis_type: String = self.client.r#type(full_key.as_str()).await?;
                        match classify_non_json_module_key(full_key, &redis_type)? {
                            NonJsonModuleKey::Stream => {
                                tracing::debug!(key = full_key, "skipping module stream key");
                                continue;
                            }
                            // The key may legitimately expire or be deleted
                            // between SCAN/JSON.GET/TYPE. There is no state
                            // left to hydrate in that case.
                            NonJsonModuleKey::Gone => continue,
                        }
                    }
                    Err(e) => return Err(e),
                };
                let Some(doc) = doc else { continue }; // gone between SCAN and GET
                let Some(key) = full_key.strip_prefix(&prefix) else {
                    continue; // unreachable under the SCAN pattern
                };
                out.push((key.to_string(), doc));
            }
            Ok(out)
        })
    }

    fn stream_range<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
        count_latest: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            validate_module_key(key)?;
            if count_latest == 0 {
                return Ok(Vec::new());
            }
            // XREVRANGE key + - COUNT n → newest-first; re-reverse below.
            let entries: Vec<(String, std::collections::HashMap<String, String>)> = self
                .client
                .xrevrange_values(
                    module_key(ns, key).as_str(),
                    "+",
                    "-",
                    Some(u64::from(count_latest)),
                )
                .await?;
            let mut out = Vec::with_capacity(entries.len());
            for (id, fields) in entries.into_iter().rev() {
                let Some(data) = fields.get("data") else {
                    // Written by something other than StreamAppend; skip.
                    tracing::error!(ns, key, id, "module stream entry without data field");
                    continue;
                };
                out.push((id, serde_json::from_str(data)?));
            }
            Ok(out)
        })
    }

    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>> {
        Box::pin(async move {
            let v: Option<i64> = self.client.get(META_SCHEMA_VERSION).await?;
            match v {
                None => Ok(None),
                Some(v) => u32::try_from(v).map(Some).map_err(|_| {
                    StoreError::Corrupt(format!("meta:schema_version out of range: {v}"))
                }),
            }
        })
    }

    fn set_schema_version(&self, version: u32) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let _: () = self
                .client
                .set(META_SCHEMA_VERSION, i64::from(version), None, None, false)
                .await?;
            Ok(())
        })
    }

    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            match self.client.bgsave::<String>().await {
                Ok(reply) => {
                    tracing::info!(reply, "BGSAVE triggered");
                    Ok(())
                }
                Err(e) if e.to_string().to_ascii_lowercase().contains("in progress") => {
                    tracing::info!("BGSAVE already in progress");
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            // Every write() has already been awaited to its EXEC ack, so
            // nothing pends client-side. One cheap round trip proves the
            // connection is live and has processed everything we sent.
            let _: Option<i64> = self.client.get(META_NEXT_ID).await?;
            Ok(())
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move { self.client.quit().await.map_err(FredError::into) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_listing_only_skips_proven_streams_or_gone_keys() {
        assert_eq!(
            classify_non_json_module_key("mod:m:history", "stream").expect("stream"),
            NonJsonModuleKey::Stream
        );
        assert_eq!(
            classify_non_json_module_key("mod:m:vanished", "none").expect("gone"),
            NonJsonModuleKey::Gone
        );

        for unexpected in ["string", "hash", "list", "set", "zset", "ReJSON-RL"] {
            let err = classify_non_json_module_key("mod:m:doc", unexpected)
                .expect_err("every other live type must fail closed");
            assert!(
                matches!(err, StoreError::Corrupt(ref message) if message.contains(unexpected)),
                "unexpected classification for {unexpected:?}: {err:?}"
            );
        }
    }

    /// The outage-detection settings must actually be on the builder: fred
    /// defaults to `default_command_timeout = 0` (commands buffer forever
    /// while disconnected), which would make the entire pause-on-outage
    /// policy unreachable. No live server needed to prove the config.
    #[test]
    fn builder_carries_outage_detection_config() {
        let builder =
            RedisStore::configured_builder("redis://127.0.0.1:6379/3").expect("builder from url");
        let perf = builder.get_performance_config();
        assert_eq!(perf.default_command_timeout, COMMAND_TIMEOUT);
        assert!(!COMMAND_TIMEOUT.is_zero(), "0 would mean 'wait forever'");
        let conn = builder.get_connection_config();
        assert_eq!(conn.unresponsive.max_timeout, Some(UNRESPONSIVE_TIMEOUT));
        let config = builder.get_config().expect("config present");
        assert_eq!(config.version, RespVersion::RESP3);
        assert!(config.fail_fast, "boot must fail fast when unreachable");
        assert!(builder.get_policy().is_some(), "reconnect policy set");
    }

    #[test]
    fn search_terms_are_escaped_literally() {
        assert_eq!(escape_search_term("Nick_42"), "Nick_42");
        assert_eq!(escape_search_term("a|b"), "a\\|b");
        assert_eq!(escape_search_term("odd)ball*"), "odd\\)ball\\*");
        assert_eq!(escape_search_term("@name:(x)"), "\\@name\\:\\(x\\)");
        assert_eq!(escape_search_term("two words"), "two\\ words");
        assert_eq!(escape_search_term("-neg"), "\\-neg");
    }
}
