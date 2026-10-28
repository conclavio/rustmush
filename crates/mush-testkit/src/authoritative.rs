//! Explicit composed-action driver for integration fixtures. Ordinary
//! `TestWorld::run` is unchanged. Only `run_and_commit_one` drives a queued
//! selector, through the real registered provider, participant and Store ACK.

use std::collections::{BTreeSet, HashMap};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use futures::executor::block_on;
use mush_core::action::PreparedCausalRef;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::{
    ActionInvocation, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalHash,
    CanonicalId, CommitToken, DefinitionRef, Effects, Engine, GOD, GameModule, IdempotencyKey,
    IdempotencyScope, IngressEnvelope, IngressPayload, IngressSurfaceVersion, ModError,
    ModuleBootData, ModuleOp, ModuleWrite, ObjId, OfferExecutionContext, OfferKeyRing, OfferSecret,
    OfferSubmission, Outbox, PrincipalId, RequestUid, ScopedIdempotencyKey,
    is_action_state_persistence_key,
};
use mush_proto::OutputEvent;
use mush_store::{
    AuthoritativeReceipt, AuthoritativeStore, IdempotencyStatus, InMemoryStore, ObjectStore,
    WriteBatch,
};
use serde_json::json;
use smol_str::SmolStr;

use crate::{TestWorld, WriteBatchView, boot_world};

type Events = Vec<(Audience, OutputEvent)>;
type Result<T> = std::result::Result<T, String>;

/// A real in-memory persistence owner beside an ordinary `TestWorld`.
/// No production module, provider or capture implementation is replaced.
pub struct AcknowledgedWorld {
    inner: TestWorld,
    store: Arc<InMemoryStore>,
    namespaces: BTreeSet<SmolStr>,
    serial: u64,
    epoch: u64,
}

impl Deref for AcknowledgedWorld {
    type Target = TestWorld;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for AcknowledgedWorld {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl AcknowledgedWorld {
    /// Register the real modules and install a deterministic fixture offer key.
    #[must_use]
    pub fn with_modules(modules: Vec<Box<dyn GameModule>>) -> Self {
        let namespaces = modules.iter().map(|module| module.name().into()).collect();
        let mut inner = TestWorld::with_modules(modules);
        install_offer_key(inner.engine_mut());
        Self {
            inner,
            store: Arc::new(InMemoryStore::new()),
            namespaces,
            serial: 0,
            epoch: 1,
        }
    }

    /// The actual Store used by this fixture's legacy and authoritative lanes.
    #[must_use]
    pub fn store(&self) -> &InMemoryStore {
        &self.store
    }

    /// Share the persisted source when rebuilding fixture objects for a cold
    /// reboot. This transfers no World or typed RAM state to the fresh engine.
    pub fn adopt_store(&mut self, source: &Self) {
        self.store = Arc::clone(&source.store);
        self.namespaces.extend(source.namespaces.iter().cloned());
        self.serial = source.serial;
        self.epoch = source.epoch.checked_add(1).expect("fixture epoch capacity");
    }

    /// Run one command and commit only its exact queued actor/action. A normal
    /// parser/admission refusal remains its original error events. Preparation
    /// refusal adds an error event and never a success receipt.
    ///
    /// # Panics
    /// A pre-existing selector/staged action is a fixture wiring error: this
    /// method never sweeps unrelated queued work.
    pub fn run_and_commit_one(
        &mut self,
        actor: ObjId,
        line: &str,
        expected_owner: &str,
        expected_action: &str,
    ) -> Events {
        assert!(
            !self.engine().has_authoritative_submit(),
            "queued fixture work"
        );
        assert!(!self.engine().has_staged_action(), "staged fixture work");
        self.flush_legacy().expect("prior fixture writes must ACK");
        let mut events = self.inner.run(actor, line);
        if !self.engine().has_authoritative_submit() {
            return events;
        }
        match self.commit_one(actor, expected_owner, expected_action) {
            Ok(committed) => events.extend(committed),
            Err(error) => {
                assert!(!error.starts_with("fixture wiring:"), "{error}");
                let mut out = Outbox::new();
                out.error(actor, error);
                events.extend(out.take());
            }
        }
        events
    }

    /// Drive an already queued selector. Callers can inspect the unchanged
    /// World and queued-only output before explicitly crossing this boundary.
    pub fn commit_one(
        &mut self,
        actor: ObjId,
        expected_owner: &str,
        expected_action: &str,
    ) -> Result<Events> {
        if self
            .engine()
            .queued_authoritative_actors()
            .collect::<Vec<_>>()
            != [actor]
        {
            return Err("fixture wiring: expected exactly one queued actor".into());
        }
        let submit = self
            .engine_mut()
            .take_authoritative_submit()
            .ok_or("queued fixture action missing")?;
        if submit.actor != actor
            || submit.action.key().owner.as_str() != expected_owner
            || submit.action.key().id.as_str() != expected_action
        {
            return Err("fixture wiring: queued action differs from the explicit selection".into());
        }
        let ingress = command_ingress(&submit, self.epoch)?;
        match block_on(
            self.store
                .authoritative_idempotency_status(ingress.idempotency(), ingress.ingress_hash()),
        )
        .map_err(message)?
        {
            IdempotencyStatus::Vacant => {}
            IdempotencyStatus::FinalizedExact(receipt) => {
                return self.receipt_events(actor, &receipt);
            }
            _ => return Err("fixture action idempotency is not vacant/finalized".into()),
        }
        let query = self
            .engine()
            .legal_action_query(ingress.principal().clone(), actor, submit.subject.clone())
            .map_err(message)?;
        let offers = self
            .engine()
            .project_legal_actions(&query)
            .map_err(message)?;
        let offer = offers
            .projected_for_action(&submit.action)
            .ok_or("the actual registered action is unavailable")?;
        let resolved = self
            .engine()
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: ingress.request(),
                    principal: ingress.principal().clone(),
                    actor,
                    idempotency: ingress.idempotency().clone(),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: offer.offer(),
                    subject: submit.subject.clone(),
                    targets: submit.targets.clone(),
                    parameters: submit.parameters.clone(),
                },
            )
            .map_err(message)?;
        let (action, resolved, guards) = resolved.into_authoritative_parts();
        let invocation = resolved.invocation();
        if invocation.request() != ingress.request()
            || invocation.principal() != ingress.principal()
            || invocation.idempotency() != ingress.idempotency()
        {
            return Err("fixture ingress/invocation identity mismatch".into());
        }
        let token = commit_token(&ingress)?;
        let (prepared, output) = self
            .engine_mut()
            .prepare_registered_composed_resolved_authoritative_action(&action, &resolved, token)
            .map_err(message)?;
        let bundle = match assemble_bundle(&ingress, &action, invocation, token, &prepared, output)
        {
            Ok(bundle) => bundle,
            Err(error) => {
                self.engine_mut().abort_prepared_transaction(prepared);
                return Err(error);
            }
        };
        self.engine_mut()
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(message)?;
        // This is the final synchronous owner/guard revalidation, not a direct
        // installation of a prepared image from earlier in the command.
        let staged = self
            .engine_mut()
            .take_staged_action()
            .map_err(message)?
            .ok_or("staged fixture action missing")?;
        let (bundle, installation) = staged.into_parts();
        let receipt = block_on(async {
            let intent = self
                .store
                .reserve_authoritative_intent((*bundle).clone())
                .await?;
            if intent.token != bundle.token() || intent.bundle_hash != bundle.bundle_hash() {
                return Err(mush_store::StoreError::Invalid(
                    "fixture intent receipt differs".into(),
                ));
            }
            self.store
                .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                .await
        })
        .map_err(message)?;
        if !receipt.matches_bundle(&bundle) {
            return Err("fixture applied receipt differs".into());
        }
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        self.engine_mut()
            .install_composed_action_announcing(installation, &mut out, &mut fx)
            .map_err(message)?;
        let mut events = out.take();
        events.extend(self.receipt_events(actor, &receipt)?);
        Ok(events)
    }

    fn receipt_events(&self, actor: ObjId, receipt: &AuthoritativeReceipt) -> Result<Events> {
        let viewer = PrincipalId::Object { id: actor };
        if !receipt.eligible_witnesses().contains(&viewer) {
            return Err("fixture wiring: receipt principal is not an eligible witness".into());
        }
        let projection = self
            .engine()
            .registry
            .action_providers()
            .ok_or("fixture wiring: provider registry unavailable")?
            .receipt_projection_table()
            .project(receipt.action(), receipt.outcome(), &viewer)
            .map_err(|e| format!("fixture wiring: receipt projection failed: {e}"))?;
        let summary = projection
            .map(|projection| projection.summary().to_owned())
            .unwrap_or_else(|| "Done.".into());
        let mut out = Outbox::new();
        out.sys_to_actor(actor, "info", summary);
        Ok(out.take())
    }

    /// ACK every ordinary dirty object and ModuleOp accumulated before the
    /// next authoritative action. Returns only the original legacy batch for
    /// assertions; authoritative rows never pass through this API.
    pub fn flush_legacy(&mut self) -> Result<WriteBatchView> {
        let view = self.inner.drain_write_batch();
        self.write_legacy(&view)?;
        Ok(view)
    }

    fn write_legacy(&mut self, view: &WriteBatchView) -> Result<()> {
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or("fixture serial capacity")?;
        let mut batch = WriteBatch {
            serial: self.serial,
            module_ops: view.module_ops.clone(),
            next_id: Some(self.world().next_id()),
            durable: true,
            ..WriteBatch::default()
        };
        self.namespaces
            .extend(view.module_ops.iter().map(|op| op.ns.clone()));
        for id in &view.dirty {
            match self.world().get(*id) {
                Some(object) => batch.upserts.push(object.clone()),
                None => batch.deletes.push(*id),
            }
        }
        block_on(self.store.write(batch)).map_err(message)?;
        Ok(())
    }

    /// Persist a direct fixture helper's real deferred legacy writes, retaining
    /// the same ordering as commands. Never accepts typed state keys.
    pub fn persist_legacy_ops(&mut self, ops: Vec<ModuleOp>) {
        let mut view = self.inner.drain_write_batch();
        view.module_ops.extend(ops);
        self.write_legacy(&view)
            .expect("direct fixture writes must ACK");
    }

    /// Drain and ACK ordinary writes while preserving the existing assertion
    /// vocabulary. Prior authoritative writes live only in the Store.
    pub fn take_module_ops(&mut self) -> Vec<ModuleOp> {
        self.flush_legacy()
            .expect("fixture writes must ACK")
            .module_ops
    }

    /// Drain and ACK the ordinary write batch.
    pub fn drain_write_batch(&mut self) -> WriteBatchView {
        self.flush_legacy().expect("fixture writes must ACK")
    }

    /// Export exact stored namespace documents for existing corruption/reboot
    /// fixtures. Typed rows remain reserved and must only be boot-hydrated.
    pub fn persisted_kv(&mut self, namespace: &str) -> Vec<(SmolStr, serde_json::Value)> {
        self.flush_legacy().expect("fixture writes must ACK");
        let docs = block_on(self.store.module_docs(namespace)).expect("fixture namespace read");
        let mut kv: Vec<_> = docs
            .docs
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect();
        kv.extend(docs.ledger.into_iter().map(|(key, text)| {
            (
                key.into(),
                serde_json::from_str(&text).expect("stored fixture ledger JSON"),
            )
        }));
        kv
    }

    /// Export all real Store namespaces, including reserved typed documents.
    pub fn persisted_boot(&mut self) -> HashMap<SmolStr, ModuleBootData> {
        self.flush_legacy().expect("fixture writes must ACK");
        let namespaces: Vec<_> = self.namespaces.iter().cloned().collect();
        namespaces
            .into_iter()
            .map(|namespace| {
                let kv = self.persisted_kv(&namespace);
                (
                    namespace,
                    ModuleBootData {
                        kv,
                        ..ModuleBootData::default()
                    },
                )
            })
            .collect()
    }

    /// Partition reserved typed rows before ordinary module callbacks. A cold
    /// reboot retaining typed data uses `adopt_store` first; that Store remains
    /// the persistence source rather than a second synthetic module-write log.
    pub fn modules_loaded(&mut self, boot: HashMap<SmolStr, ModuleBootData>) {
        self.try_modules_loaded(boot)
            .expect("fixture boot data must load");
    }

    /// Fallible version for corruption fixtures.
    pub fn try_modules_loaded(
        &mut self,
        mut boot: HashMap<SmolStr, ModuleBootData>,
    ) -> std::result::Result<(), ModError> {
        for namespace in &self.namespaces {
            if !boot.contains_key(namespace) {
                let stored = block_on(self.store.module_docs(namespace))
                    .map_err(|e| ModError::msg(e.to_string()))?;
                if stored
                    .docs
                    .iter()
                    .any(|(key, _)| is_action_state_persistence_key(key))
                {
                    return Err(ModError::msg(
                        "fixture boot omitted an acknowledged typed namespace",
                    ));
                }
            }
        }
        self.flush_legacy().map_err(ModError::msg)?;
        let mut typed = Vec::new();
        let mut legacy = Vec::new();
        for (namespace, data) in &mut boot {
            self.namespaces.insert(namespace.clone());
            let current = block_on(self.store.module_docs(namespace))
                .map_err(|e| ModError::msg(e.to_string()))?;
            let mut ordinary = Vec::new();
            let mut typed_keys = BTreeSet::new();
            for (key, value) in std::mem::take(&mut data.kv) {
                if is_action_state_persistence_key(&key) {
                    let stored = block_on(self.store.module_json_get(namespace, &key))
                        .map_err(|e| ModError::msg(e.to_string()))?;
                    if stored.as_ref() != Some(&value) {
                        return Err(ModError::msg(
                            "typed boot must use the actual acknowledged fixture Store",
                        ));
                    }
                    typed_keys.insert(key.to_string());
                    typed.push(ActionStateBootRecord::new(namespace.clone(), key, value));
                } else {
                    ordinary.push((key, value));
                }
            }
            data.kv = ordinary;
            let current_typed: BTreeSet<_> = current
                .docs
                .iter()
                .filter(|(key, _)| is_action_state_persistence_key(key))
                .map(|(key, _)| key.clone())
                .collect();
            if current_typed != typed_keys {
                return Err(ModError::msg(
                    "fixture boot omitted acknowledged typed rows",
                ));
            }
            let mut present: BTreeSet<_> = data.kv.iter().map(|(key, _)| key.to_string()).collect();
            present.extend(data.ledger.iter().map(|(key, _)| key.to_string()));
            for key in current
                .docs
                .into_iter()
                .map(|(key, _)| key)
                .chain(current.ledger.into_iter().map(|(key, _)| key))
            {
                if !is_action_state_persistence_key(&key) && !present.contains(&key) {
                    legacy.push(ModuleOp {
                        ns: namespace.clone(),
                        op: ModuleWrite::Del { key: key.into() },
                    });
                }
            }
            for (key, value) in &data.kv {
                legacy.push(ModuleOp {
                    ns: namespace.clone(),
                    op: ModuleWrite::JsonSet {
                        key: key.clone(),
                        value: value.clone(),
                    },
                });
            }
            for (key, text) in &data.ledger {
                let value = serde_json::from_str(text).map_err(|e| ModError::msg(e.to_string()))?;
                legacy.push(ModuleOp {
                    ns: namespace.clone(),
                    op: ModuleWrite::JsonSet {
                        key: key.clone(),
                        value,
                    },
                });
            }
        }
        if !typed.is_empty() {
            self.engine_mut()
                .hydrate_action_state(typed)
                .map_err(|e| ModError::msg(e.to_string()))?;
        }
        self.write_legacy(&WriteBatchView {
            dirty: BTreeSet::new(),
            module_ops: legacy,
        })
        .map_err(ModError::msg)?;
        self.inner.try_modules_loaded(boot)
    }

    /// Recompose the same fixture objects with the actual Store retained and
    /// typed rows hydrated before any module boot callback.
    pub fn reboot_into(
        self,
        modules: Vec<Box<dyn GameModule>>,
        boot: HashMap<SmolStr, ModuleBootData>,
    ) -> std::result::Result<Self, ModError> {
        let Self {
            inner,
            store,
            namespaces,
            serial,
            epoch,
        } = self;
        let mut world = inner.engine.world;
        let connected: Vec<_> = world
            .iter()
            .map(|o| o.id)
            .filter(|id| world.is_connected(*id))
            .collect();
        for id in connected {
            world.set_connected(id, false);
        }
        let (_, registry) = boot_world();
        let mut engine = Engine::with_modules(world, registry, modules)?;
        install_offer_key(&mut engine);
        let mut next = Self {
            inner: TestWorld {
                engine: Box::new(engine),
                next_conn: inner.next_conn,
            },
            store,
            namespaces,
            serial,
            epoch: epoch
                .checked_add(1)
                .ok_or_else(|| ModError::msg("fixture epoch capacity"))?,
        };
        next.try_modules_loaded(boot)?;
        next.connect(GOD);
        Ok(next)
    }
}

fn install_offer_key(engine: &mut Engine) {
    engine.set_offer_keys(
        OfferKeyRing::new(OfferSecret::new(1, [0x63; 32]).expect("fixture secret"), [])
            .expect("fixture offer key ring"),
    );
}
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn command_ingress(submit: &AuthoritativeSubmit, epoch: u64) -> Result<IngressEnvelope> {
    let principal = PrincipalId::Object { id: submit.actor };
    let digest = CanonicalHash::for_canonical_value(
        "cmd.legal_action.v1",
        1,
        &json!({
            "actor":submit.actor,"action":submit.action,"subject":submit.subject,
            "targets":submit.targets,"parameters":submit.parameters,"key":submit.key,
        }),
        64 * 1024,
    )
    .map_err(message)?;
    let bytes: [u8; 16] = digest
        .as_bytes()
        .iter()
        .copied()
        .take(16)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "fixture request digest length")?;
    let request = RequestUid::from_bytes(bytes);
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: submit.action.key().owner.clone(),
            namespace: CanonicalId::new(submit.action.key().id.as_str()).map_err(message)?,
        },
        epoch,
        IdempotencyKey::Request(request),
    );
    IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(CanonicalId::new("cmd.legal_action").map_err(message)?, 1)
            .map_err(message)?,
        IngressPayload::structured(json!({"action":submit.action,"subject":submit.subject,
            "targets":submit.targets,"parameters":submit.parameters}))
        .map_err(message)?,
        idempotency,
    )
    .map_err(message)
}
fn commit_token(ingress: &IngressEnvelope) -> Result<CommitToken> {
    let digest = CanonicalHash::for_canonical_value("authoritative_commit_token.v1",1,&json!({
        "request":ingress.request(),"idempotency":ingress.idempotency(),"ingress_hash":ingress.ingress_hash(),
    }),8*1024).map_err(message)?;
    let bytes: [u8; 16] = digest
        .as_bytes()
        .iter()
        .copied()
        .take(16)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "fixture commit digest length")?;
    Ok(CommitToken::from_bytes(bytes))
}

fn assemble_bundle(
    ingress: &IngressEnvelope,
    action: &DefinitionRef,
    invocation: &ActionInvocation,
    token: CommitToken,
    prepared: &mush_core::composed::PreparedComposedTxn,
    output: mush_core::PreparedAuthoritativeAction,
) -> Result<AuthoritativeBundle> {
    let provider = prepared.owner().clone();
    let writes = prepared.authoritative_replay_writes();
    let (outcome, resolution, mut definitions, mut witnesses, mut durable_outbox, semantic_events) =
        output.into_parts();
    definitions.extend([action.clone(), outcome.schema().clone()]);
    definitions.sort();
    definitions.dedup();
    if witnesses.is_empty() {
        witnesses.push(invocation.principal().clone());
    }
    witnesses.sort();
    witnesses.dedup();
    durable_outbox.sort_by_key(mush_core::DurableOutboxIntent::ordinal);
    let cause = PreparedCausalRef::new(invocation.request(), None, 0).map_err(message)?;
    let resolution = resolution.map(|r| r.bind(token));
    let plan_hash = CanonicalHash::for_canonical_value(
        "authoritative_action_plan.v1",
        1,
        &json!({
            "request_hash":invocation.request_hash(),"action":action,"provider":provider,
            "cause":cause,"definitions":definitions,"writes":writes,"outcome":outcome,
            "resolution":resolution,"semantic_events":semantic_events,
            "eligible_witnesses":witnesses,"durable_outbox":durable_outbox,
        }),
        mush_core::MAX_AUTHORITATIVE_BUNDLE_BYTES,
    )
    .map_err(message)?;
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request: invocation.request(),
        idempotency: invocation.idempotency().clone(),
        ingress_hash: ingress.ingress_hash(),
        request_hash: invocation.request_hash(),
        plan_hash,
        action: action.clone(),
        provider,
        cause,
        definitions,
        writes,
        outcome,
        resolution,
        semantic_events,
        eligible_witnesses: witnesses,
        durable_outbox,
    })
    .map_err(message)
}
