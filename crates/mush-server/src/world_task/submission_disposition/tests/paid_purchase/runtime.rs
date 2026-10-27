//! The actual server boot/loop, with a bounded in-process gateway recorder.
use super::{TestResult, store::PurchaseFaultStore};
use crate::gateway_sink::GatewaySink;
use crate::world_task::{
    Gateway, LegalActionReadError, PersistHandles, ServerDeps, WorldHandle, WorldReadError,
};
use mush_config::{config::Config, ipguard::IpGuard, metrics::Metrics};
use mush_core::{
    CommitToken, ControlMsg, DefinitionRef, Engine, Inbound, ObjId, OfferRouteTable, PerceivedRef,
};
use mush_ipc::{ActionSend, DetachCause, GatewayAction, SendTarget};
use mush_proto::{OutputEvent, ProjectedActionReceipt, ProjectedActionStatus};
use mush_store::{AuthoritativeStore, BoundIdempotencyStatus, ObjectStore};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tacsim::composition::mind_commerce::MindCommerce;
use tokio::time::{Instant, timeout};

pub(in crate::world_task::submission_disposition::tests) const STEP: Duration =
    Duration::from_secs(120);
/// How long the harness waits before resubmitting a request the world
/// answered with its typed, retryable `Busy`; every resubmission shares the
/// one [`STEP`] guard of the call that started it.
const BUSY_RETRY: Duration = Duration::from_millis(50);

/// Sleep out one `Busy` answer, or fail once the guard `end` has passed.
async fn wait_busy(end: Instant, what: &str) -> TestResult {
    if Instant::now() >= end {
        return Err(format!("the world stayed busy past the finite guard: {what}").into());
    }
    tokio::time::sleep(BUSY_RETRY).await;
    Ok(())
}

pub(super) fn now() -> TestResult<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .try_into()?)
}

#[derive(Default)]
struct Recorded {
    lines: Vec<String>,
    overflow: bool,
}
#[derive(Default)]
struct Recorder(Mutex<Recorded>);
impl Recorder {
    fn drain(&self) -> TestResult<Vec<String>> {
        let mut state = self.0.lock().map_err(|_| "gateway recorder poisoned")?;
        if state.overflow {
            return Err("bounded gateway recorder overflow".into());
        }
        Ok(std::mem::take(&mut state.lines))
    }
}
impl GatewaySink for Recorder {
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        if let Ok(mut state) = self.0.lock() {
            for (_, event) in sends {
                let text = mush_testkit::text_of(&event);
                if text.len() > 16 * 1024 || state.lines.len() >= 4096 {
                    state.overflow = true;
                } else {
                    state.lines.push(text);
                }
            }
        }
    }
    fn set_flood_exempt(&self, _: u64, _: bool) {}
    fn is_socket(&self) -> bool {
        false
    }
    fn close_session(&self, _: u64, _: DetachCause) {}
    fn close_web_sessions(&self, _: Vec<u64>, _: Option<OutputEvent>, _: DetachCause) {}
    fn close_char_connections(&self, _: u64) -> Vec<u64> {
        Vec::new()
    }
    fn structured_conns_of(&self, _: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }
    fn detach_conn_local(&self, _: u64) {}
    fn session_counts(&self) -> (i64, i64) {
        (2, 2)
    }
}

pub(in crate::world_task::submission_disposition::tests) struct Running {
    handles: Option<PersistHandles>,
    store: Arc<PurchaseFaultStore>,
    recorder: Arc<Recorder>,
    routes: OfferRouteTable,
    pub exchange: ObjId,
}
impl Running {
    pub fn handle(&self) -> TestResult<&WorldHandle> {
        Ok(&self
            .handles
            .as_ref()
            .ok_or("server already stopped")?
            .handle)
    }

    /// 17g C1a: the booted registry's offer route table, read-only. C1c
    /// compares it with a real `build_engine` boot over the same store.
    pub(in crate::world_task::submission_disposition::tests) fn routes(&self) -> &OfferRouteTable {
        &self.routes
    }

    pub fn drain_output(&self) -> TestResult {
        self.recorder.drain()?;
        Ok(())
    }

    /// Mirrors the actual build_engine sequence, with only the explicit
    /// opt-in roster and fixed test key material substituted. No owner state
    /// or command output is installed here.
    pub fn boot<'a>(
        config: &'a Config,
        store: Arc<PurchaseFaultStore>,
        commerce: Option<&'a MindCommerce>,
    ) -> std::pin::Pin<Box<impl std::future::Future<Output = TestResult<Self>> + 'a>> {
        // Keep the complete boot state out of the calling test/timeout futures.
        // Like production build_engine, this remains lazy until first polled.
        Self::bounded_boot_future(|| Self::boot_inner(config, store, commerce))
    }

    // Check the concrete future during test-binary code generation, including
    // boot paths that an earlier failing assertion would never reach.
    #[inline(never)]
    fn bounded_boot_future<F: std::future::Future>(
        make: impl FnOnce() -> F,
    ) -> std::pin::Pin<Box<F>> {
        const {
            assert!(
                std::mem::size_of::<F>() <= 16 * 1024,
                "server story boot future exceeds the configured 16 KiB bound"
            );
        }
        Box::pin(make())
    }

    async fn boot_inner(
        config: &Config,
        store: Arc<PurchaseFaultStore>,
        commerce: Option<&MindCommerce>,
    ) -> TestResult<Self> {
        crate::persist::reconcile_authoritative_for_writable_boot(&*store).await?;
        store.bootstrap_writable().await?;
        mush_store::run_migrations(&*store, &mush_store::core_migrations()).await?;
        let identity = crate::accounts_identity::hydrate(&*store).await?;
        let staff = Arc::new(crate::staff::StaffQueue::new());
        let mut registry = crate::world_task::base_registry();
        crate::staff::register_server_commands_with_cap(
            &mut registry,
            &staff,
            config.game.max_characters as usize,
        )?;
        let llm = Arc::new(mush_llm::LlmService::from_config(
            &crate::config_ext::llm_config(&config.llm)?,
        )?);
        let modules = match commerce {
            Some(commerce) => crate::boot::game_modules_with_mind_commerce(config, &llm, commerce)?,
            None => crate::boot::game_modules(config, &llm)?,
        };
        // These actual boot aggregates cross store awaits. Retain their
        // ownership on the heap while the guarded boot future is pending.
        let composition = Box::new(Engine::compose_modules(registry, modules)?);
        let mut world = Box::new(crate::persist::load_or_seed_world(&*store).await?);
        crate::timeline::hydrate_world_timeline(&mut world, &*store, config.game.game_clock()?)
            .await?;
        let mut engine = Box::new(Engine::from_composition(*world, *composition));
        engine.set_offer_keys(mush_core::offer::OfferKeyRing::new(
            mush_core::offer::OfferSecret::new(1, [0x51; 32])?,
            [],
        )?);
        engine.set_resolution_keys(mush_core::resolution::ResolutionKeyRing::new(
            mush_core::resolution::ResolutionSecret::new(1, [0x52; 32])?,
            [],
        )?);
        engine.set_decision_keys(mush_core::decision::DecisionKeyRing::new(
            mush_core::decision::DecisionSecret::new(1, [0x53; 32])?,
            [],
        )?);
        let identity: Arc<dyn mush_core::AccountIdentity> = Arc::new(identity);
        engine.set_account_identity(identity.clone());
        *engine
            .world
            .module_state_mut::<mush_sim::AccountOracle>()
            .ok_or("actual Sim oracle")? = mush_sim::AccountOracle::new(identity);
        *engine
            .world
            .module_state_mut::<mush_sim::tick::PulseConfig>()
            .ok_or("actual pulse owner")? = mush_sim::tick::PulseConfig {
            ground_ms: config.tacsim.ground_pulse_ms,
            space_ms: config.tacsim.space_pulse_ms,
        };
        mush_tactics::ground::configure(&mut engine.world, config.tacsim.ground_cell_cm)?;
        crate::modules::boot_modules(&mut engine, &*store).await?;
        let room_key = tacsim::port::room_key(
            &mush_core::mechanics::CanonicalId::new("neighborhood.ceres_freeport")?,
            "neighborhood.ceres_freeport.room.c07_exchange",
        )?;
        let exchange = tacsim::port::apply::room_by_key(&engine.world, &room_key)
            .ok_or("actual Ceres exchange room")?;
        let routes = engine
            .registry
            .action_providers()
            .ok_or("actual authoritative providers")?
            .offer_route_table();
        let recorder = Arc::new(Recorder::default());
        let sink = recorder.clone();
        let deps = ServerDeps {
            store: store.clone(),
            guard: Arc::new(IpGuard::new()),
            staff,
            llm: Some(llm.clone()),
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        };
        let handles = crate::world_task::spawn_persist_with_gateway_production(
            *engine,
            deps,
            Some(llm),
            2,
            move |_| -> Gateway { sink },
        )?;
        Ok(Self {
            handles: Some(handles),
            store,
            recorder,
            routes,
            exchange,
        })
    }

    /// Attach `actor` on its own connection and prove the world tracks it.
    ///
    /// The attach itself is lifecycle custody: the world holds it behind a
    /// pending commit and never refuses it, so only the two proofs after it
    /// are re-asked while the world answers `Busy` (see [`Self::offer`]).
    pub async fn connect(&self, actor: ObjId) -> TestResult {
        if !self
            .handle()?
            .inbound(Inbound::ConnUp {
                conn: actor.0.into(),
                actor,
                caps_structured: true,
                from_linkdead: false,
            })
            .await
        {
            return Err("server connection admission gone".into());
        }
        let end = Instant::now() + STEP;
        loop {
            match timeout(STEP, self.handle()?.barrier_result()).await? {
                Ok(()) => break,
                Err(WorldReadError::Busy) => wait_busy(end, "the connect barrier").await?,
                Err(error) => return Err(error.into()),
            }
        }
        loop {
            match self.handle()?.connection_ready(actor.0.into(), actor).await {
                Ok(true) => return Ok(()),
                Ok(false) => return Err("actual connection is not ready".into()),
                Err(WorldReadError::Busy) => wait_busy(end, "the connection proof").await?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// One typed line as `actor`, answered by everything it printed. The line
    /// is admitted once; only the ordering barrier behind it is re-asked while
    /// the world answers `Busy` (a full defer queue or waiter cap).
    pub async fn command(&self, actor: ObjId, line: String) -> TestResult<Vec<String>> {
        self.recorder.drain()?;
        if !self
            .handle()?
            .inbound(Inbound::Cmd {
                conn: actor.0.into(),
                actor,
                line,
            })
            .await
        {
            return Err("server command admission gone".into());
        }
        let end = Instant::now() + STEP;
        loop {
            match timeout(STEP, self.handle()?.barrier_result()).await? {
                Ok(()) => return self.recorder.drain(),
                Err(WorldReadError::Busy) => wait_busy(end, "the command barrier").await?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// One public offer as `actor`, resubmitted while the world answers
    /// `Busy`, every attempt within the one [`STEP`] guard.
    ///
    /// `Busy` is the world task's typed refusal of work that meets a pending
    /// commit (`RefusalReason::CommitPending`). Every durable flush of a
    /// dirtied world gates the next mutation behind the writer's receipt, and
    /// every periodic module tick is armed against the loaded world's clock at
    /// engine construction, so the first Timer wake after a boot fires them
    /// all and its flush is the process's first commit. A
    /// `WorldMsg::AuthoritativeAction` that arrives in that window is refused
    /// at once, with no defer-queue wait (a read gets 250 ms), and the gateway
    /// turns the same answer into "The game is busy; try that action again":
    /// this harness is that player.
    /// Nothing was submitted under a `Busy`. The in-process path mints a
    /// fresh request sequence per call and no intent exists under one until
    /// the writer owns it, so a resubmission cannot double-apply.
    pub async fn offer(
        &self,
        actor: ObjId,
        verb: &str,
        parameters: Value,
    ) -> TestResult<ProjectedActionReceipt> {
        eprintln!("M7_SERVER_PURCHASE_OFFER verb={verb} event=begin");
        let end = Instant::now() + STEP;
        let mut busy = 0u32;
        loop {
            let stage = match self.offer_once(actor, verb, &parameters).await? {
                Ok(receipt) => return Ok(receipt),
                Err(stage) => stage,
            };
            busy += 1;
            // Bounded diagnostics: the stage and the count, no bodies.
            eprintln!(
                "M7_SERVER_PURCHASE_OFFER verb={verb} event=busy stage={stage} attempt={busy}"
            );
            if Instant::now() >= end {
                return Err(format!(
                    "actual public offer stayed busy past the finite guard: verb={verb} stage={stage} attempts={busy}"
                )
                .into());
            }
            tokio::time::sleep(BUSY_RETRY).await;
        }
    }

    /// One attempt of [`Self::offer`]: the legal-action read, then the
    /// submission. `Err` is a harness or custody failure; `Ok(Err(stage))` is
    /// the world's `Busy` at that stage.
    async fn offer_once(
        &self,
        actor: ObjId,
        verb: &str,
        parameters: &Value,
    ) -> TestResult<Result<ProjectedActionReceipt, &'static str>> {
        let supplied = parameters.as_object().ok_or("offer parameters object")?;
        let offers = match timeout(
            STEP,
            self.handle()?
                .legal_actions(actor.0.into(), actor, PerceivedRef::SelfActor),
        )
        .await?
        {
            Ok(offers) => offers,
            Err(LegalActionReadError::Busy) => return Ok(Err("legal_actions")),
            Err(error) => return Err(error.into()),
        };
        let mut matching = offers.into_iter().filter(|offer| {
            offer.presentation().verb == verb
                && offer
                    .parameters()
                    .parameters()
                    .iter()
                    .all(|p| supplied.contains_key(p.id.as_str()))
                && offer.parameters().parameters().len() == supplied.len()
        });
        let offer = matching
            .next()
            .ok_or_else(|| format!("actual offer unavailable: {verb}"))?;
        if matching.next().is_some() {
            return Err("ambiguous actual legal offer".into());
        }
        let expected_action = self
            .routes
            .get(offer.offer().route_id())
            .ok_or("actual selected offer route")?
            .clone();
        let response = timeout(
            STEP,
            self.handle()?.submit_legal_action(GatewayAction {
                conn: actor.0.into(),
                actor,
                offered_action: offer.offer(),
                subject: offer.subject().clone(),
                targets: Vec::new(),
                parameters: parameters.clone(),
            }),
        )
        .await?;
        match response {
            // Finished is an actual finalized authoritative receipt. Its optional
            // provider projection is presentation, not a second success bit.
            // Callers independently join the retained business rows to this commit.
            ActionSend::Finished(receipt) => {
                self.verify_committed(actor, &expected_action, &receipt)
                    .await?;
                eprintln!(
                    "M7_SERVER_PURCHASE_OFFER verb={verb} event=committed projected={}",
                    receipt.result().is_some()
                );
                Ok(Ok(receipt))
            }
            ActionSend::Owned(reference) => {
                let end = Instant::now() + STEP;
                loop {
                    match self
                        .handle()?
                        .action_status(actor.0.into(), actor, &reference)
                        .await
                    {
                        Ok(ProjectedActionStatus::Committed(receipt)) => {
                            if receipt.status() != &reference {
                                return Err("owned public receipt reference changed".into());
                            }
                            self.verify_committed(actor, &expected_action, &receipt)
                                .await?;
                            eprintln!(
                                "M7_SERVER_PURCHASE_OFFER verb={verb} event=committed projected={}",
                                receipt.result().is_some()
                            );
                            return Ok(Ok(receipt));
                        }
                        // The status read waits in the defer queue behind the
                        // very commit it asks after; its `Busy` is the queue's
                        // bound, not an answer, so the poll goes on.
                        Ok(ProjectedActionStatus::Pending { .. })
                        | Ok(ProjectedActionStatus::Unavailable { .. })
                        | Err(WorldReadError::Busy) => {}
                        Ok(_) => {
                            return Err("owned action did not retain a committed success".into());
                        }
                        Err(error) => return Err(error.into()),
                    }
                    if Instant::now() >= end {
                        return Err("owned public action hang guard".into());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            // The world refused before custody: the preflight stayed transient
            // to its bound, the readiness read or the action itself met the
            // pending commit, or the writer refused the intent.
            ActionSend::Busy => Ok(Err("submit")),
            other => {
                let disposition = match other {
                    ActionSend::Accepted => "accepted_without_durable_receipt",
                    ActionSend::Gone => "gone",
                    ActionSend::Rejected => "rejected",
                    ActionSend::Finished(_) | ActionSend::Owned(_) | ActionSend::Busy => {
                        "unexpected"
                    }
                };
                Err(format!(
                    "actual public offer did not establish successful custody: verb={verb} disposition={disposition}"
                ).into())
            }
        }
    }

    // Read-only corroboration of the public success against the exact durable
    // receipt. The boot registry supplies the full expected DefinitionRef;
    // neither the display verb nor an optional projection grants authority.
    async fn verify_committed(
        &self,
        actor: ObjId,
        expected_action: &DefinitionRef,
        projected: &ProjectedActionReceipt,
    ) -> TestResult {
        let viewer = mush_core::PrincipalId::Object { id: actor };
        let key =
            crate::action_coordinator::ActionCoordinator::status_key(&viewer, projected.status())?;
        let BoundIdempotencyStatus::FinalizedExact(actual) = self
            .store
            .bound_authoritative_idempotency_status(&key)
            .await?
        else {
            return Err("public committed receipt lacks exact finalized storage".into());
        };
        let token = CommitToken::from_hex(projected.commit())?;
        if actual.action() != expected_action
            || actual.token() != token
            || actual.idempotency() != &key
            || !actual.eligible_witnesses().contains(&viewer)
        {
            return Err("public committed receipt disagrees with exact action or custody".into());
        }
        Ok(())
    }

    pub async fn stop(&mut self) -> TestResult {
        let handles = self.handles.as_mut().ok_or("server already stopped")?;
        if !handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
        {
            return Err("server shutdown channel closed".into());
        }
        timeout(STEP, &mut handles.world_join).await??;
        timeout(STEP, &mut handles.writer_join).await??;
        timeout(STEP, &mut handles.outbox_join).await??;
        self.handles.take();
        Ok(())
    }

    pub async fn crash(&mut self) -> TestResult {
        let handles = self.handles.as_mut().ok_or("server already stopped")?;
        handles.world_join.abort();
        handles.writer_join.abort();
        handles.outbox_join.abort();
        for task in [&mut handles.writer_join, &mut handles.outbox_join] {
            let result = timeout(STEP, task).await?;
            if let Err(error) = result
                && !error.is_cancelled()
            {
                return Err(error.into());
            }
        }
        let result = timeout(STEP, &mut handles.world_join).await?;
        if let Err(error) = result
            && !error.is_cancelled()
        {
            return Err(error.into());
        }
        self.handles.take();
        Ok(())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        // Unblock only the fixture's own store and stop only its own tasks.
        self.store.heal();
        if let Some(handles) = &self.handles {
            handles.world_join.abort();
            handles.writer_join.abort();
            handles.outbox_join.abort();
        }
    }
}

pub(in crate::world_task::submission_disposition::tests) async fn rows(
    store: &PurchaseFaultStore,
    owner: &str,
    family: &str,
) -> TestResult<Vec<Value>> {
    Ok(store
        .module_json_list(owner)
        .await?
        .into_iter()
        .filter_map(|(_, row)| {
            (row.get("family")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str)
                == Some(family))
            .then(|| row.get("value").cloned())
            .flatten()
        })
        .collect())
}
pub(in crate::world_task::submission_disposition::tests) async fn one_row(
    store: &PurchaseFaultStore,
    owner: &str,
    family: &str,
) -> TestResult<Value> {
    let [row]: [Value; 1] = rows(store, owner, family)
        .await?
        .try_into()
        .map_err(|_| format!("expected exactly one retained {owner}/{family} row"))?;
    Ok(row)
}
pub(in crate::world_task::submission_disposition::tests) fn field<'a>(
    value: &'a Value,
    name: &str,
) -> TestResult<&'a Value> {
    value
        .get(name)
        .ok_or_else(|| format!("required retained field {name}").into())
}
pub(super) async fn wall_after(deadline: i64) -> TestResult {
    let end = Instant::now() + STEP;
    while now()? <= deadline {
        if Instant::now() >= end {
            return Err("wall expiry wait exceeds finite guard".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
