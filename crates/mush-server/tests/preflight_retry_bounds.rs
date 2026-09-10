//! WS06-S8: the mandatory preflight retry loop is BOUNDED, and giving up on it
//! is answered rather than leaked.
//!
//! The loop exists because a transient store answer proves nothing: it is
//! neither "this request already owns an intent" nor "no intent exists", so
//! neither `Owned` nor `Busy` may be published on the strength of one. Before
//! this step the all-in-one arm spun on that forever - a task per submission,
//! doubling to a 5 s backoff and never stopping - so a store outage grew an
//! unbounded pile of futures nobody could answer.
//!
//! These proofs drive the **all-in-one** arm
//! ([`WorldHandle::submit_legal_action`]), whose answer at the bound is
//! `ActionSend::Busy`. That is safe only here: the handle mints a fresh
//! request sequence per call, so no intent under this submission's identity
//! can exist yet and a retype cannot double-apply one -
//! `busy_leaves_nothing_behind_to_double_apply` proves exactly that. The split
//! boundary's arm is bounded by the same pair of constants but answers with
//! *retention* instead, because its gateway still holds a replay copy; it is
//! proved in `ipc_authoritative_action.rs`
//! (`a_permanently_transient_preflight_retains_and_then_stops_at_the_rearm_bound`).
//!
//! [`WorldHandle::submit_legal_action`]: mush_server::world_task::WorldHandle::submit_legal_action

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{
    AuthoritativeBundle, CanonicalHash, CommitToken, ControlMsg, Engine, GOD, IdempotencyScope,
    Inbound, ObjId, PerceivedRef, ScopedIdempotencyKey,
};
use mush_ipc::{ActionSend, DetachCause, GatewayAction, SendTarget};
use mush_proto::OutputEvent;
use mush_server::gateway_sink::GatewaySink;
use mush_server::quarantine::QuarantineSpool;
use mush_server::staff::StaffQueue;
use mush_server::world_task::{
    self, Gateway, PersistHandles, ServerDeps, spawn_persist_with_gateway,
};
use mush_store::{
    AccountRec, AuthoritativeCheckpointReceipt, AuthoritativeDiscardReceipt,
    AuthoritativeEpochRetirement, AuthoritativeIntentPage, AuthoritativeOutboxAttemptResult,
    AuthoritativeOutboxRecord, AuthoritativeReceipt, AuthoritativeRetentionUsage,
    AuthoritativeScanCursor, AuthoritativeStore, BanRec, BootSkip, BoundIdempotencyStatus,
    BundleStatus, DiscordBanRec, DiscordCodeRecord, DiscordLinkRec, IdempotencyStatus,
    InMemoryStore, IntentReceipt, ModuleDocs, ObjectStore, PendingAuthoritativeIntent, SshKeyRec,
    StoreError, TokenRecord, WorldSnapshot, WriteBatch, WriteReceipt,
};

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

/// `action_coordinator::MAX_PREFLIGHT_ATTEMPTS`, which is crate-private: the
/// bound is a property of the server, not of its callers, so the test restates
/// the number it is asserting rather than importing it.
const EXPECTED_ATTEMPTS: u32 = 5;

/// `action_coordinator::PREFLIGHT_RETRY_WINDOW`, restated for the same reason.
const EXPECTED_WINDOW: Duration = Duration::from_secs(5);

/// The world loop needs a sink; nothing here inspects delivery.
struct BlackholeGateway;

impl GatewaySink for BlackholeGateway {
    fn transmit(&self, _sends: Vec<(SendTarget, Arc<OutputEvent>)>) {}

    fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<OutputEvent>,
        _cause: DetachCause,
    ) {
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

/// An [`ObjectStore`] whose authoritative **preflight lookup** can be made to
/// answer transiently - forever, or for a fixed number of lookups, and with a
/// controlled latency per answer. Everything else (boot, the world's writes,
/// the rest of the authoritative protocol) is the ordinary in-memory store.
///
/// It returns `Some(self)` from [`ObjectStore::authoritative_store`] on
/// purpose: the trait default is `None`, and a double that forgets this would
/// send every caller down the "store lacks the capability" branch instead of
/// through the retry loop under test.
struct TransientPreflightStore {
    inner: InMemoryStore,
    /// How many further preflight lookups answer transiently; [`u32::MAX`]
    /// means "forever".
    transient_left: AtomicU32,
    /// How long each transient answer takes to arrive.
    answer_delay_ms: AtomicU64,
    /// Preflight lookups since the fault was armed.
    lookups: AtomicU32,
}

impl TransientPreflightStore {
    fn new() -> Arc<TransientPreflightStore> {
        Arc::new(TransientPreflightStore {
            inner: InMemoryStore::new(),
            transient_left: AtomicU32::new(0),
            answer_delay_ms: AtomicU64::new(0),
            lookups: AtomicU32::new(0),
        })
    }

    /// Answer transiently for the rest of the test, each answer taking
    /// `delay`. Resets the lookup count so a proof measures its own loop.
    fn arm_forever(&self, delay: Duration) {
        self.answer_delay_ms
            .store(delay.as_millis() as u64, Ordering::SeqCst);
        self.transient_left.store(u32::MAX, Ordering::SeqCst);
        self.lookups.store(0, Ordering::SeqCst);
    }

    /// Answer transiently `count` times, then honestly.
    fn arm_transient(&self, count: u32) {
        self.answer_delay_ms.store(0, Ordering::SeqCst);
        self.transient_left.store(count, Ordering::SeqCst);
        self.lookups.store(0, Ordering::SeqCst);
    }

    fn heal(&self) {
        self.transient_left.store(0, Ordering::SeqCst);
        self.lookups.store(0, Ordering::SeqCst);
    }

    fn lookups(&self) -> u32 {
        self.lookups.load(Ordering::SeqCst)
    }

    fn authoritative(&self) -> &dyn AuthoritativeStore {
        self.inner
            .authoritative_store()
            .expect("the in-memory store is authoritative")
    }
}

impl AuthoritativeStore for TransientPreflightStore {
    /// The one method the fault owns: the mandatory read-only preflight
    /// lookup. Everything else on both traits is the ordinary in-memory store.
    fn authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
        ingress_hash: CanonicalHash,
    ) -> BoxFuture<'a, Result<IdempotencyStatus, StoreError>> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        let left = self.transient_left.load(Ordering::SeqCst);
        if left == 0 {
            return self
                .authoritative()
                .authoritative_idempotency_status(key, ingress_hash);
        }
        if left != u32::MAX {
            self.transient_left.fetch_sub(1, Ordering::SeqCst);
        }
        let delay = Duration::from_millis(self.answer_delay_ms.load(Ordering::SeqCst));
        Box::pin(async move {
            // A store that is slow to answer transiently is the case the
            // wall-clock window exists for; a zero delay is the case the
            // attempt ceiling exists for.
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            // Transient by construction: `is_transient()` is true for it, so
            // the coordinator must read it as "unproven", never as "no intent
            // exists".
            Err(StoreError::AuthoritativeEpochContention)
        })
    }

    fn reserve_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        self.authoritative().reserve_authoritative_intent(bundle)
    }

    fn bound_authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
    ) -> BoxFuture<'a, Result<BoundIdempotencyStatus, StoreError>> {
        self.authoritative()
            .bound_authoritative_idempotency_status(key)
    }

    fn reconcile_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeReceipt, StoreError>> {
        self.authoritative()
            .reconcile_authoritative(token, expected_bundle_hash)
    }

    fn checkpoint_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeCheckpointReceipt, StoreError>> {
        self.authoritative()
            .checkpoint_authoritative(token, expected_bundle_hash)
    }

    fn discard_quarantined_authoritative_intent(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeDiscardReceipt, StoreError>> {
        self.authoritative()
            .discard_quarantined_authoritative_intent(token, expected_bundle_hash, evidence_hash)
    }

    fn restore_quarantined_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        self.authoritative()
            .restore_quarantined_authoritative_intent(bundle, evidence_hash)
    }

    fn pending_authoritative_outbox(
        &self,
    ) -> BoxFuture<'_, Result<Vec<AuthoritativeOutboxRecord>, StoreError>> {
        self.authoritative().pending_authoritative_outbox()
    }

    fn begin_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempts: u32,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        self.authoritative().begin_authoritative_outbox_attempt(
            item_id,
            expected_payload_hash,
            expected_attempts,
        )
    }

    fn complete_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempt: u32,
        result: AuthoritativeOutboxAttemptResult,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        self.authoritative().complete_authoritative_outbox_attempt(
            item_id,
            expected_payload_hash,
            expected_attempt,
            result,
        )
    }

    fn note_authoritative_outbox_unroutable(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_passes: u32,
        error: String,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        self.authoritative().note_authoritative_outbox_unroutable(
            item_id,
            expected_payload_hash,
            expected_passes,
            error,
        )
    }

    fn retire_authoritative_epochs<'a>(
        &'a self,
        scope: &'a IdempotencyScope,
        minimum_accepted_epoch: u64,
    ) -> BoxFuture<'a, Result<AuthoritativeEpochRetirement, StoreError>> {
        self.authoritative()
            .retire_authoritative_epochs(scope, minimum_accepted_epoch)
    }

    fn authoritative_status(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<BundleStatus, StoreError>> {
        self.authoritative()
            .authoritative_status(token, expected_bundle_hash)
    }

    fn pending_authoritative_intents(
        &self,
    ) -> BoxFuture<'_, Result<Vec<PendingAuthoritativeIntent>, StoreError>> {
        self.authoritative().pending_authoritative_intents()
    }

    fn scan_authoritative_intents(
        &self,
        cursor: Option<AuthoritativeScanCursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<AuthoritativeIntentPage, StoreError>> {
        self.authoritative()
            .scan_authoritative_intents(cursor, limit)
    }

    fn authoritative_retention_generation(&self) -> BoxFuture<'_, Result<u64, StoreError>> {
        self.authoritative().authoritative_retention_generation()
    }

    fn repair_authoritative_retention(
        &self,
        expected_generation: u64,
        usage: AuthoritativeRetentionUsage,
    ) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.authoritative()
            .repair_authoritative_retention(expected_generation, usage)
    }
}

impl ObjectStore for TransientPreflightStore {
    fn authoritative_store(&self) -> Option<&dyn AuthoritativeStore> {
        Some(self)
    }

    fn bootstrap_writable(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.bootstrap_writable()
    }

    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        self.inner.load_world()
    }

    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        self.inner.write(batch)
    }

    fn create_account(&self, rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.create_account(rec)
    }

    fn get_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>> {
        self.inner.get_account(account)
    }

    fn link_character<'a>(
        &'a self,
        account: &'a str,
        id: ObjId,
        name: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        self.inner.link_character(account, id, name)
    }

    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>> {
        self.inner.list_accounts()
    }

    fn set_account_password<'a>(
        &'a self,
        account: &'a str,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_password(account, hash)
    }

    fn set_account_ssh_keys<'a>(
        &'a self,
        account: &'a str,
        keys: &'a [SshKeyRec],
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_ssh_keys(account, keys)
    }

    fn set_account_discord<'a>(
        &'a self,
        account: &'a str,
        link: Option<&'a DiscordLinkRec>,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_discord(account, link)
    }

    fn discord_user_reserve<'a>(
        &'a self,
        user_id: u64,
        account: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.discord_user_reserve(user_id, account)
    }

    fn discord_user_release(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.inner.discord_user_release(user_id)
    }

    fn account_by_discord_user(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<AccountRec>, StoreError>> {
        self.inner.account_by_discord_user(user_id)
    }

    fn discord_code_put(
        &self,
        code: String,
        rec: DiscordCodeRecord,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.discord_code_put(code, rec)
    }

    fn discord_code_take<'a>(
        &'a self,
        code: &'a str,
    ) -> BoxFuture<'a, Result<Option<DiscordCodeRecord>, StoreError>> {
        self.inner.discord_code_take(code)
    }

    fn discord_ban_put(&self, rec: DiscordBanRec) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.discord_ban_put(rec)
    }

    fn discord_ban_get(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<DiscordBanRec>, StoreError>> {
        self.inner.discord_ban_get(user_id)
    }

    fn discord_ban_del(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.inner.discord_ban_del(user_id)
    }

    fn set_ban<'a>(
        &'a self,
        ip: &'a str,
        until_unix: Option<i64>,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        self.inner.set_ban(ip, until_unix, reason)
    }

    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>> {
        self.inner.get_bans()
    }

    fn clear_ban<'a>(&'a self, ip: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.clear_ban(ip)
    }

    fn token_put(&self, hash: String, rec: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.token_put(hash, rec)
    }

    fn token_get<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>> {
        self.inner.token_get(hash)
    }

    fn token_touch<'a>(
        &'a self,
        hash: &'a str,
        now: i64,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.token_touch(hash, now)
    }

    fn token_del<'a>(&'a self, hash: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.token_del(hash)
    }

    fn tokens_del_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<u64, StoreError>> {
        self.inner.tokens_del_account(account)
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        self.inner.module_json_get(ns, key)
    }

    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        self.inner.module_json_list(ns)
    }

    fn module_docs<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        self.inner.module_docs(ns)
    }

    fn module_docs_from<'a>(
        &'a self,
        ns: &'a str,
        skip: &'a BootSkip,
    ) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        self.inner.module_docs_from(ns, skip)
    }

    fn module_keys<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<Vec<String>, StoreError>> {
        self.inner.module_keys(ns)
    }

    fn module_doc_text<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, StoreError>> {
        self.inner.module_doc_text(ns, key)
    }

    fn module_purge_archived<'a>(
        &'a self,
        ns: &'a str,
        keys: Vec<String>,
    ) -> BoxFuture<'a, Result<usize, StoreError>> {
        self.inner.module_purge_archived(ns, keys)
    }

    fn ledger_archive(&self) -> Option<&mush_store::LedgerArchive> {
        self.inner.ledger_archive()
    }

    fn stream_range<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
        count_latest: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        self.inner.stream_range(ns, key, count_latest)
    }

    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>> {
        self.inner.get_schema_version()
    }

    fn set_schema_version(&self, version: u32) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.set_schema_version(version)
    }

    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.bgsave()
    }

    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.drain()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.close()
    }
}

/// Boot the real all-in-one world over the fault store, with the tactics
/// module composed so there is a genuine signed offer to submit.
fn boot(store: Arc<dyn ObjectStore>, metrics: &Arc<Metrics>) -> PersistHandles {
    let mut engine = Engine::with_modules(
        mush_core::World::new_minimal(),
        world_task::base_registry(),
        vec![Box::new(mush_tactics::TacticsModule::new())],
    )
    .expect("tactics module composition");
    engine.set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x68; 32]).expect("test offer secret"),
            [],
        )
        .expect("test offer key ring"),
    );
    spawn_persist_with_gateway(
        engine,
        ServerDeps {
            store,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::clone(metrics),
            quarantine_spool: QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        |_| -> Gateway { Arc::new(BlackholeGateway) },
    )
}

const CONN: u64 = 0x6868;

/// Attach `CONN` and project the tactics `brace` offer for it. The offer is
/// what makes the submission reach preflight at all: ingress that cannot be
/// resolved against the route table is refused long before the store.
async fn brace_offer(handle: &world_task::WorldHandle) -> GatewayAction {
    assert!(
        handle
            .inbound(Inbound::ConnUp {
                conn: CONN,
                actor: GOD,
                caps_structured: true,
                from_linkdead: false,
            })
            .await
    );
    assert!(
        handle.barrier().await,
        "connection must become authoritative"
    );
    let offer = handle
        .legal_actions(CONN, GOD, PerceivedRef::SelfActor)
        .await
        .expect("legal-action projection")
        .into_iter()
        .find(|offer| offer.presentation().verb == "brace")
        .expect("tactics brace offer");
    GatewayAction {
        conn: CONN,
        actor: GOD,
        offered_action: offer.offer(),
        subject: offer.subject().clone(),
        targets: Vec::new(),
        parameters: serde_json::json!({}),
    }
}

async fn shutdown(handles: PersistHandles) {
    let PersistHandles {
        handle,
        world_join,
        writer_join,
        outbox_join,
        ..
    } = handles;
    assert!(handle.inbound(Inbound::Control(ControlMsg::Shutdown)).await);
    drop(handle);
    tokio::time::timeout(Duration::from_secs(10), world_join)
        .await
        .expect("world shutdown timed out")
        .expect("world task panicked");
    tokio::time::timeout(Duration::from_secs(10), writer_join)
        .await
        .expect("writer shutdown timed out")
        .expect("writer task panicked");
    tokio::time::timeout(Duration::from_secs(10), outbox_join)
        .await
        .expect("outbox shutdown timed out")
        .expect("outbox task panicked");
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// A store that fails fast and forever: the ATTEMPT ceiling is what stops the
/// loop, and the submission is answered instead of hanging.
///
/// Against the pre-step code this test never finishes - the future retries
/// every 5 s for the life of the process - so the whole submission is wrapped
/// in a timeout well under the runtime's patience.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_in_one_attempt_ceiling() {
    let store = TransientPreflightStore::new();
    let metrics = Arc::new(Metrics::new());
    let handles = boot(store.clone(), &metrics);
    let action = brace_offer(&handles.handle).await;

    store.arm_forever(Duration::ZERO);
    let started = std::time::Instant::now();
    let sent = tokio::time::timeout(
        Duration::from_secs(20),
        handles.handle.submit_legal_action(action),
    )
    .await
    .expect("the bounded loop must answer; an unbounded one never does");
    let elapsed = started.elapsed();

    assert_eq!(
        sent,
        ActionSend::Busy,
        "an exhausted preflight is refused as bounded custody, never as an ownership claim"
    );
    assert_eq!(
        store.lookups(),
        EXPECTED_ATTEMPTS,
        "the ceiling counts lookups, not sleeps"
    );
    // Four sleeps of 100/200/400/800 ms sit inside the window, so the ceiling
    // - not the window - is what fired.
    assert!(
        elapsed < EXPECTED_WINDOW,
        "a fail-fast store must be bound by attempts, not by the clock: {elapsed:?}"
    );
    assert_eq!(
        metrics.snapshot().coordinator_preflight_deferred_total,
        1,
        "the deferral is the only operational signal the caller's Busy carries"
    );

    shutdown(handles).await;
}

/// A store that is merely SLOW to answer transiently: the attempt ceiling
/// would let this run for minutes, so the wall-clock window is what stops it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_in_one_window_governs() {
    let store = TransientPreflightStore::new();
    let metrics = Arc::new(Metrics::new());
    let handles = boot(store.clone(), &metrics);
    let action = brace_offer(&handles.handle).await;

    // Two seconds per answer: attempts 1..3 spend 6.3 s of the 5 s window
    // between them, so the loop stops at the third with two attempts unspent.
    store.arm_forever(Duration::from_secs(2));
    let started = std::time::Instant::now();
    let sent = tokio::time::timeout(
        Duration::from_secs(30),
        handles.handle.submit_legal_action(action),
    )
    .await
    .expect("the bounded loop must answer; an unbounded one never does");
    let elapsed = started.elapsed();

    assert_eq!(sent, ActionSend::Busy);
    assert!(
        store.lookups() < EXPECTED_ATTEMPTS,
        "the window must fire first on a slow store, with attempts left over: {} used",
        store.lookups()
    );
    assert!(
        elapsed >= EXPECTED_WINDOW,
        "the window is a floor as well as a ceiling; a shorter give-up is a different bug: {elapsed:?}"
    );
    assert!(
        elapsed < EXPECTED_WINDOW * 3,
        "the window bounds the whole loop, not each attempt: {elapsed:?}"
    );
    assert_eq!(metrics.snapshot().coordinator_preflight_deferred_total, 1);

    shutdown(handles).await;
}

/// The loop is still a RETRY loop: a hiccup that clears inside the bound is
/// ridden out, and the submission reaches durable ownership with nothing
/// deferred.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_then_success_still_owns() {
    let store = TransientPreflightStore::new();
    let metrics = Arc::new(Metrics::new());
    let handles = boot(store.clone(), &metrics);
    let action = brace_offer(&handles.handle).await;

    store.arm_transient(3);
    let sent = tokio::time::timeout(
        Duration::from_secs(20),
        handles.handle.submit_legal_action(action),
    )
    .await
    .expect("three transients are well inside the bound");

    // The all-in-one arm runs the action to its exact committed receipt, so
    // ownership shows up as `Finished` rather than the split arm's `Owned`.
    assert!(
        matches!(sent, ActionSend::Finished(_)),
        "a cleared hiccup must still reach durable ownership: {sent:?}"
    );
    assert_eq!(
        store.lookups(),
        4,
        "three transient lookups and the one that answered"
    );
    assert_eq!(
        metrics.snapshot().coordinator_preflight_deferred_total,
        0,
        "nothing was deferred: the loop did its job"
    );

    shutdown(handles).await;
}

/// Why `Busy` is honest on this arm: the refusal happens before any intent is
/// reserved, and every submission mints its own request sequence, so the same
/// offer re-submitted after the store heals commits exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn busy_leaves_nothing_behind_to_double_apply() {
    let store = TransientPreflightStore::new();
    let metrics = Arc::new(Metrics::new());
    let handles = boot(store.clone(), &metrics);
    let action = brace_offer(&handles.handle).await;

    store.arm_forever(Duration::ZERO);
    let refused = tokio::time::timeout(
        Duration::from_secs(20),
        handles.handle.submit_legal_action(action.clone()),
    )
    .await
    .expect("the bounded loop must answer");
    assert_eq!(refused, ActionSend::Busy);

    store.heal();
    let retyped = tokio::time::timeout(
        Duration::from_secs(20),
        handles.handle.submit_legal_action(action),
    )
    .await
    .expect("a healed store answers at once");
    assert!(
        matches!(retyped, ActionSend::Finished(_)),
        "the retype must commit: the refusal left no intent to collide with: {retyped:?}"
    );
    assert!(handles.handle.barrier().await, "the receipt must settle");

    let stance = store
        .inner
        .module_json_get("tactics", "action-state/v1/stance/1")
        .await
        .expect("read persisted tactics stance")
        .expect("the retype's post-image must be durable");
    assert_eq!(
        stance["value"]["revision"], 1,
        "exactly one brace was applied, not the refused one and the retype"
    );
    assert_eq!(metrics.snapshot().coordinator_preflight_deferred_total, 1);

    shutdown(handles).await;
}
