//! Real store forwarding; only the read-only preflight lookup is faulted.
use futures::future::BoxFuture;
use mush_core::{
    AuthoritativeBundle, CanonicalHash, CommitToken, IdempotencyScope, ObjId, ScopedIdempotencyKey,
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
use std::sync::atomic::{AtomicU32, Ordering};

pub(super) struct TransientPreflightStore {
    inner: InMemoryStore,
    transient_left: AtomicU32,
    lookups: AtomicU32,
}
impl TransientPreflightStore {
    pub(super) fn new(count: u32) -> Self {
        Self {
            inner: InMemoryStore::new(),
            transient_left: AtomicU32::new(count),
            lookups: AtomicU32::new(0),
        }
    }
    pub(super) fn lookups(&self) -> u32 {
        self.lookups.load(Ordering::SeqCst)
    }
    pub(super) fn arm(&self, count: u32) {
        self.transient_left.store(count, Ordering::SeqCst);
    }
    fn authoritative(&self) -> &dyn AuthoritativeStore {
        &self.inner
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
        let transient = self
            .transient_left
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if transient {
            Box::pin(async { Err(StoreError::AuthoritativeEpochContention) })
        } else {
            self.authoritative()
                .authoritative_idempotency_status(key, ingress_hash)
        }
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

    fn put_tempban_lift(
        &self,
        lift: mush_store::TempbanLift,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.put_tempban_lift(lift)
    }
    fn get_tempban_lifts(&self) -> BoxFuture<'_, Result<Vec<mush_store::TempbanLift>, StoreError>> {
        self.inner.get_tempban_lifts()
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
