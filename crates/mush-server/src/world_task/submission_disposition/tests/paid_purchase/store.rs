//! Actual durable store forwarding. Only the selected paid Grant writer phase is faulted.
#[path = "registry_timing.rs"]
mod registry_timing;

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
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    ReconcileUnknown,
    AbsentBeforeIntent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Exact {
    pub token: CommitToken,
    pub hash: CanonicalHash,
    pub action: mush_core::DefinitionRef,
    pub operation: CommitToken,
    pub quote: CommitToken,
}

#[derive(Clone, Debug)]
pub(super) struct Event {
    pub exact: Exact,
    pub reserved: bool,
    pub finalized: bool,
}

struct Arm {
    quote: CommitToken,
    operation: Option<CommitToken>,
    fault: Fault,
    selected: Option<Exact>,
}

#[derive(Default)]
struct State {
    arm: Option<Arm>,
    events: Vec<Event>,
}

pub(in crate::world_task::submission_disposition::tests) struct PurchaseFaultStore {
    inner: InMemoryStore,
    state: Mutex<State>,
    released: AtomicBool,
    release: Notify,
    registry_timing: registry_timing::Observer,
}

impl PurchaseFaultStore {
    pub(in crate::world_task::submission_disposition::tests) fn new() -> Self {
        Self {
            inner: InMemoryStore::new(),
            state: Mutex::new(State::default()),
            released: AtomicBool::new(true),
            release: Notify::new(),
            registry_timing: registry_timing::Observer::new(),
        }
    }

    pub(super) fn arm(&self, quote: CommitToken, fault: Fault) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("fault state poisoned"))?;
        if !self.released.load(Ordering::SeqCst) {
            return Err(invalid("another fixture fault remains armed"));
        }
        state.arm = Some(Arm {
            quote,
            operation: None,
            fault,
            selected: None,
        });
        self.released.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub(super) fn heal(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.release.notify_waiters();
    }

    pub(super) fn selected(&self) -> Result<Option<Exact>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| invalid("fault state poisoned"))?
            .arm
            .as_ref()
            .and_then(|a| a.selected.clone()))
    }

    pub(super) fn events(&self) -> Result<Vec<Event>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| invalid("fault state poisoned"))?
            .events
            .clone())
    }

    fn authoritative(&self) -> &dyn AuthoritativeStore {
        &self.inner
    }

    fn identify(bundle: &AuthoritativeBundle) -> Result<Option<Exact>, StoreError> {
        let key = bundle.action().key();
        if key.owner.as_str() != "market"
            || key.kind.as_str() != "action"
            || !matches!(
                key.id.as_str(),
                "purchase_original_mind" | "settle_original_mind" | "refund_original_mind"
            )
        {
            return Ok(None);
        }
        if bundle.action().version() != 1 {
            return Err(invalid("unexpected paid action version"));
        }
        let rows = bundle
            .writes()
            .iter()
            .filter_map(|write| {
                let mush_core::ReplayWrite::ModuleJsonSet {
                    namespace, value, ..
                } = write
                else {
                    return None;
                };
                (namespace.as_str() == "market"
                    && value
                        .get("family")
                        .and_then(|f| f.get("id"))
                        .and_then(serde_json::Value::as_str)
                        == Some("market_mind_purchase"))
                .then(|| value.get("value"))
                .flatten()
            })
            .collect::<Vec<_>>();
        let [row] = rows.as_slice() else {
            return Err(invalid(
                "paid bundle must contain its one actual operation row",
            ));
        };
        let operation = serde_json::from_value(
            row.get("operation")
                .ok_or_else(|| invalid("operation missing"))?
                .clone(),
        )
        .map_err(|_| invalid("invalid operation token in actual paid bundle"))?;
        let quote = serde_json::from_value(
            row.get("quote")
                .and_then(|q| q.get("publication"))
                .ok_or_else(|| invalid("quote identity missing"))?
                .clone(),
        )
        .map_err(|_| invalid("invalid quote token in actual paid bundle"))?;
        Ok(Some(Exact {
            token: bundle.token(),
            hash: bundle.bundle_hash(),
            action: bundle.action().clone(),
            operation,
            quote,
        }))
    }

    fn observe(&self, exact: &Exact) -> Result<Option<Fault>, StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("fault state poisoned"))?;
        if let Some(old) = state
            .events
            .iter()
            .find(|event| event.exact.token == exact.token)
        {
            if old.exact != *exact {
                return Err(invalid("same token changed its exact paid bundle"));
            }
        } else {
            if state.events.len() >= 128 {
                return Err(invalid("paid fixture event bound exceeded"));
            }
            state.events.push(Event {
                exact: exact.clone(),
                reserved: false,
                finalized: false,
            });
        }
        let Some(arm) = &mut state.arm else {
            return Ok(None);
        };
        if arm.quote != exact.quote {
            return Ok(None);
        }
        if exact.action.key().id.as_str() == "purchase_original_mind" {
            if arm.operation.is_some_and(|old| old != exact.operation) {
                return Err(invalid(
                    "one acknowledged quote produced two funding operations",
                ));
            }
            arm.operation = Some(exact.operation);
            return Ok(None);
        }
        if arm.operation != Some(exact.operation)
            || exact.action.key().id.as_str() != "settle_original_mind"
        {
            return Ok(None);
        }
        if arm.selected.is_none() {
            arm.selected = Some(exact.clone());
        }
        // Definitely absent retries can have fresh tokens. Keep rejecting
        // only this quote/operation's actual Grant attempts before storage;
        // the real driver must choose Refund using its real advanced clock.
        Ok(
            (arm.fault == Fault::AbsentBeforeIntent || arm.selected.as_ref() == Some(exact))
                .then_some(arm.fault),
        )
    }

    fn mark(
        &self,
        token: CommitToken,
        hash: CanonicalHash,
        finalized: bool,
    ) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("fault state poisoned"))?;
        if let Some(event) = state
            .events
            .iter_mut()
            .find(|event| event.exact.token == token)
        {
            if event.exact.hash != hash {
                return Err(invalid("crossed paid receipt identity"));
            }
            event.reserved = true;
            event.finalized |= finalized;
        }
        Ok(())
    }

    fn reserve_selected(
        &self,
        bundle: AuthoritativeBundle,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        Box::pin(async move {
            self.registry_timing.reserve_enter(&bundle);
            let exact = Self::identify(&bundle)?;
            let fault = exact
                .as_ref()
                .map(|e| self.observe(e))
                .transpose()?
                .flatten();
            if fault == Some(Fault::AbsentBeforeIntent) {
                // This call has not touched the actual store. The real writer
                // still queries exact Absent and performs its ordinary abort.
                loop {
                    let notified = self.release.notified();
                    if self.released.load(Ordering::SeqCst) {
                        break;
                    }
                    notified.await;
                }
                return Err(invalid("fixture rejects this exact unreserved grant"));
            }
            let receipt = self.inner.reserve_authoritative_intent(bundle).await?;
            self.registry_timing
                .reserved(receipt.token, receipt.bundle_hash);
            self.mark(receipt.token, receipt.bundle_hash, false)?;
            Ok(receipt)
        })
    }

    fn reconcile_selected(
        &self,
        token: CommitToken,
        hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeReceipt, StoreError>> {
        Box::pin(async move {
            self.registry_timing.reconcile_enter(token, hash);
            let selected = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| invalid("fault state poisoned"))?;
                state.arm.as_ref().is_some_and(|arm| {
                    arm.fault == Fault::ReconcileUnknown
                        && arm
                            .selected
                            .as_ref()
                            .is_some_and(|exact| exact.token == token && exact.hash == hash)
                })
            };
            if selected && !self.released.load(Ordering::SeqCst) {
                return Err(StoreError::AuthoritativeEpochContention);
            }
            let receipt = self.inner.reconcile_authoritative(token, hash).await?;
            self.registry_timing.committed(token, hash);
            self.mark(token, hash, true)?;
            Ok(receipt)
        })
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::Invalid(message.to_owned())
}
impl AuthoritativeStore for PurchaseFaultStore {
    fn authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
        ingress_hash: CanonicalHash,
    ) -> BoxFuture<'a, Result<IdempotencyStatus, StoreError>> {
        self.authoritative()
            .authoritative_idempotency_status(key, ingress_hash)
    }

    fn reserve_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        self.reserve_selected(bundle)
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
        self.reconcile_selected(token, expected_bundle_hash)
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

impl ObjectStore for PurchaseFaultStore {
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
