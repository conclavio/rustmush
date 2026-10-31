//! The actual server boot/loop, with a bounded in-process gateway recorder.
use super::{TestResult, store::PurchaseFaultStore};
use crate::gateway_sink::GatewaySink;
use crate::world_task::{Gateway, PersistHandles, ServerDeps, WorldHandle, WorldReadError};
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

pub(super) fn now() -> TestResult<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .try_into()?)
}

/// 17g C2a (C2 plan §4): each recorded line keeps the `SendTarget` it was
/// sent to, so a test can tell the actor's own lines from a bystander's.
///
/// 17g C3a (the C3 plan §4.1): the structured events too, beside the lines,
/// so a typed test reads its `OutputEvent::InvokeResult` and query `Reply`
/// (`Running::invoke`, `query`) and what reached each connection
/// (`event_seen`). They are drained on their own: a test that never asks
/// for them keeps only the newest `MAX_EVENTS`, and they never set the
/// lines' overflow.
#[derive(Default)]
struct Recorded {
    lines: Vec<(SendTarget, String)>,
    events: std::collections::VecDeque<(SendTarget, Arc<OutputEvent>)>,
    overflow: bool,
}
/// The structured events the recorder keeps between drains (the oldest go
/// first).
const MAX_EVENTS: usize = 4096;
#[derive(Default)]
struct Recorder(Mutex<Recorded>);
impl Recorder {
    /// 17g C3a: every structured event since the last drain, with its target.
    fn drain_events(&self) -> TestResult<Vec<(SendTarget, Arc<OutputEvent>)>> {
        let mut state = self.0.lock().map_err(|_| "gateway recorder poisoned")?;
        Ok(state.events.drain(..).collect())
    }
    fn drain_targeted(&self) -> TestResult<Vec<(SendTarget, String)>> {
        let mut state = self.0.lock().map_err(|_| "gateway recorder poisoned")?;
        if state.overflow {
            return Err("bounded gateway recorder overflow".into());
        }
        Ok(std::mem::take(&mut state.lines))
    }
    fn drain(&self) -> TestResult<Vec<String>> {
        Ok(self
            .drain_targeted()?
            .into_iter()
            .map(|(_, line)| line)
            .collect())
    }
}
/// 17g C2a: one command's output split by recipient. `own` holds the lines
/// sent to the actor (`SendTarget::Conn` or `Char` of the actor; the harness
/// connects each actor on the connection numbered as its object), `others`
/// every other line (another connection or character, the room fan-out
/// `All`, a relay), and `to` the target of each `others` line, index for
/// index, so a test can ask what reached one observer (`reaching`) without
/// counting unrelated traffic to God, the clerk or the operator.
#[derive(Debug, Default)]
pub(in crate::world_task::submission_disposition::tests) struct Seen {
    pub own: Vec<String>,
    pub others: Vec<String>,
    pub to: Vec<SendTarget>,
}
impl Seen {
    /// Append a later `Seen` (an awaited service line, a settle window).
    pub fn extend(&mut self, more: Seen) {
        self.own.extend(more.own);
        self.others.extend(more.others);
        self.to.extend(more.to);
    }
    /// The lines that reached `who`'s connection: sent to its character, to
    /// its connection (numbered as its object) or to every connection.
    pub fn reaching(&self, who: ObjId) -> Vec<&str> {
        let id = u64::from(who.0);
        self.others
            .iter()
            .zip(&self.to)
            .filter(|(_, target)| {
                matches!(target, SendTarget::Conn(n) | SendTarget::Char(n) if *n == id)
                    || matches!(target, SendTarget::All)
            })
            .map(|(line, _)| line.as_str())
            .collect()
    }
}
impl GatewaySink for Recorder {
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        if let Ok(mut state) = self.0.lock() {
            for (target, event) in sends {
                let text = mush_testkit::text_of(&event);
                if state.events.len() >= MAX_EVENTS {
                    state.events.pop_front();
                }
                state.events.push_back((target.clone(), event));
                if text.len() > 16 * 1024 || state.lines.len() >= 4096 {
                    state.overflow = true;
                } else {
                    state.lines.push((target, text));
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
    owner_reads: Option<(usize, usize)>,
    /// 17g C3a: the next typed request's correlation id (`invoke`, `query`).
    next_id: std::sync::atomic::AtomicU64,
    /// 17g C3a: the booted registry's panel feeds, in registration order (what
    /// the gateway's manifest lists).
    panels: Vec<String>,
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

    /// 17g C2e: the booted registry's owner-read (services, grants) counts
    /// (`Registry::owner_read_counts`), read-only, so a command test proves
    /// its read is the composed one.
    pub(in crate::world_task::submission_disposition::tests) const fn owner_read_counts(
        &self,
    ) -> Option<(usize, usize)> {
        self.owner_reads
    }

    /// 17g C3a: the booted registry's panel feed names, in registration
    /// order: the feeds the client manifest lists.
    pub(in crate::world_task::submission_disposition::tests) fn panel_feeds(&self) -> &[String] {
        &self.panels
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
        let owner_reads = engine.registry.owner_read_counts();
        let panels = engine
            .registry
            .panel_feeds()
            .iter()
            .map(|(feed, _)| feed.to_string())
            .collect();
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
            crate::world_task::DEFAULT_WORLD_CLOCK_CHECKPOINT_S,
            move |_| -> Gateway { sink },
        )?;
        Ok(Self {
            handles: Some(handles),
            store,
            recorder,
            routes,
            owner_reads,
            next_id: std::sync::atomic::AtomicU64::new(1 << 20),
            panels,
            exchange,
        })
    }

    /// Attach `actor` on its own connection and prove the world tracks it.
    ///
    /// The attach itself is lifecycle custody: the world holds it behind a
    /// pending commit and never refuses it. The two proofs after it are
    /// reads, refused with a typed `Busy` once they outlive the defer bound
    /// behind a commit, so each is re-asked, bounded like `legal_actions`
    /// (50 tries, each under `STEP`, 50 ms apart).
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
        self.barrier_retried().await?;
        let mut tries = 0;
        loop {
            match self.handle()?.connection_ready(actor.0.into(), actor).await {
                Ok(true) => return Ok(()),
                Ok(false) => return Err("actual connection is not ready".into()),
                Err(WorldReadError::Busy) if tries < 49 => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// The ordering barrier behind everything sent so far, re-asked while
    /// the world answers `Busy` (a full defer queue or waiter cap), bounded
    /// like `legal_actions` (50 tries, each under `STEP`, 50 ms apart).
    async fn barrier_retried(&self) -> TestResult {
        let mut tries = 0;
        loop {
            match timeout(STEP, self.handle()?.barrier_result()).await? {
                Ok(()) => return Ok(()),
                Err(WorldReadError::Busy) if tries < 49 => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub async fn command(&self, actor: ObjId, line: String) -> TestResult<Vec<String>> {
        self.send_command(actor, line).await?;
        self.recorder.drain()
    }

    /// 17g C2a (C2 plan §4): `command`, with the output split by recipient
    /// (`Seen`), so an observer-rule test can prove no line of the actor's
    /// command reaches anyone else.
    pub async fn command_seen(&self, actor: ObjId, line: String) -> TestResult<Seen> {
        self.send_command(actor, line).await?;
        self.take_seen(actor)
    }

    /// 17g C2a: everything sent since the last drain, split for `actor`,
    /// without sending a command (a driver's commit-bound confirmation, a
    /// service's later line).
    pub fn take_seen(&self, actor: ObjId) -> TestResult<Seen> {
        let own = |target: &SendTarget| matches!(target, SendTarget::Conn(id) | SendTarget::Char(id) if *id == u64::from(actor.0));
        let mut seen = Seen::default();
        for (target, text) in self.recorder.drain_targeted()? {
            if own(&target) {
                seen.own.push(text);
            } else {
                seen.others.push(text);
                seen.to.push(target);
            }
        }
        Ok(seen)
    }

    async fn send_command(&self, actor: ObjId, line: String) -> TestResult {
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
        self.barrier_retried().await
    }

    /// 17g C3a (the C3 plan §4.1): one typed invoke from `actor`, as the
    /// gateway forwards it: `Inbound::Invoke` on the actor's own connection
    /// (numbered as its object, as `connect` attached it) with a fresh
    /// correlation id, then the `OutputEvent::InvokeResult` carrying that id
    /// on that connection, within `STEP`. Lines and events from before are
    /// dropped first (as `command` drops lines); the lines and events the
    /// invoke caused stay for `take_seen` and `event_seen`, the invoke result
    /// among them.
    pub async fn invoke(
        &self,
        actor: ObjId,
        request: mush_proto::InvokeRequest,
    ) -> TestResult<mush_proto::InvokeResult> {
        self.recorder.drain()?;
        self.recorder.drain_events()?;
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !self
            .handle()?
            .inbound(Inbound::Invoke {
                conn: actor.0.into(),
                actor,
                id,
                request,
            })
            .await
        {
            return Err("server invoke admission gone".into());
        }
        self.barrier_retried().await?;
        let found = self
            .await_event(actor, |event| match event {
                OutputEvent::InvokeResult { id: got, result } if *got == id => Some(result.clone()),
                _ => None,
            })
            .await?;
        found.ok_or_else(|| format!("no invoke result {id} for #{}", actor.0).into())
    }

    /// 17g C3a: one panel query from `actor`, as the gateway forwards it:
    /// `Inbound::Query` named `name` with `params` on the actor's own
    /// connection and a fresh id, then the event inside the `Reply` carrying
    /// that id on that connection (a keyframe, or the handler's `Sys`
    /// refusal), within `STEP`. Like `invoke`, it drops what came before and
    /// keeps what it caused.
    pub async fn query(&self, actor: ObjId, name: &str, params: Value) -> TestResult<OutputEvent> {
        self.recorder.drain()?;
        self.recorder.drain_events()?;
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !self
            .handle()?
            .inbound(Inbound::Query {
                conn: actor.0.into(),
                actor,
                id,
                name: name.into(),
                args: params,
            })
            .await
        {
            return Err("server query admission gone".into());
        }
        self.barrier_retried().await?;
        let found = self
            .await_event(actor, |event| match event {
                OutputEvent::Reply { id: got, event } if *got == id => Some((**event).clone()),
                _ => None,
            })
            .await?;
        found.ok_or_else(|| format!("no reply {id} for #{}", actor.0).into())
    }

    /// The first recorded event on `actor`'s own connection that `pick`
    /// answers, within `STEP`; every event read is kept for `event_seen`.
    async fn await_event<T>(
        &self,
        actor: ObjId,
        pick: impl Fn(&OutputEvent) -> Option<T>,
    ) -> TestResult<Option<T>> {
        let own = u64::from(actor.0);
        let end = Instant::now() + STEP;
        let mut kept = Vec::new();
        let found = loop {
            let events = self.recorder.drain_events()?;
            let hit = events.iter().find_map(|(target, event)| {
                matches!(target, SendTarget::Conn(n) if *n == own)
                    .then(|| pick(event))
                    .flatten()
            });
            kept.extend(events);
            if hit.is_some() || Instant::now() >= end {
                break hit;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let mut state = self
            .recorder
            .0
            .lock()
            .map_err(|_| "gateway recorder poisoned")?;
        for event in kept.into_iter().rev() {
            state.events.push_front(event);
        }
        Ok(found)
    }

    /// 17g C3a (the C3 plan §4.1): every structured event since the last
    /// drain, split for `actor` as `take_seen` splits lines: `own` holds the
    /// JSON of each event sent to the actor's connection or character,
    /// `others` (with `to`) the JSON of every other event, so a test asks
    /// what reached one observer (`Seen::reaching`) and runs
    /// `assert_private` over the events' JSON.
    pub fn event_seen(&self, actor: ObjId) -> TestResult<Seen> {
        let own = |target: &SendTarget| matches!(target, SendTarget::Conn(id) | SendTarget::Char(id) if *id == u64::from(actor.0));
        let mut seen = Seen::default();
        for (target, event) in self.recorder.drain_events()? {
            let json = serde_json::to_string(&*event)?;
            if own(&target) {
                seen.own.push(json);
            } else {
                seen.others.push(json);
                seen.to.push(target);
            }
        }
        Ok(seen)
    }

    /// The actor's projected legal actions. 17g C2a (C1a's residual, C2 plan
    /// §4): a read that lands while the world commits another action answers
    /// a typed `Busy` before reading anything, so it is retried, bounded (50
    /// tries, each under `STEP`, 20 ms apart), never treated as a failure or
    /// an empty projection.
    async fn legal_actions(
        &self,
        actor: ObjId,
    ) -> TestResult<Vec<mush_core::ProjectedLegalAction>> {
        let mut tries = 0;
        loop {
            match timeout(
                STEP,
                self.handle()?
                    .legal_actions(actor.0.into(), actor, PerceivedRef::SelfActor),
            )
            .await?
            {
                Ok(offers) => return Ok(offers),
                Err(crate::world_task::LegalActionReadError::Busy) if tries < 49 => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// One public offer as `actor`: the legal-action read, then the
    /// submission, each resubmitted while the world answers `Busy`.
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
    /// this harness is that player. Nothing was submitted under a `Busy`; the
    /// in-process path mints a fresh request sequence per call and no intent
    /// exists under one until the writer owns it, so a resubmission cannot
    /// double-apply.
    pub async fn offer(
        &self,
        actor: ObjId,
        verb: &str,
        parameters: Value,
    ) -> TestResult<ProjectedActionReceipt> {
        eprintln!("M7_SERVER_PURCHASE_OFFER verb={verb} event=begin");
        let supplied = parameters.as_object().ok_or("offer parameters object")?;
        let offers = self.legal_actions(actor).await?;
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
        // 17g C2 close: `Busy` is the all-in-one arm's typed refusal before
        // durable ownership (bounded custody full, or the world committing
        // another action, such as a boot-time service step, under CI load).
        // The handle mints a fresh request sequence per call, so no intent
        // under the refused submission can exist and a retry is exact
        // (`preflight_retry_bounds.rs`). Retried like `legal_actions`,
        // bounded (50 tries, each under `STEP`, 50 ms apart); every other
        // disposition is answered below as before.
        let mut tries = 0;
        let response = loop {
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
                ActionSend::Busy if tries < 49 => {
                    tries += 1;
                    eprintln!("M7_SERVER_PURCHASE_OFFER verb={verb} event=busy_retry try={tries}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                response => break response,
            }
        };
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
                Ok(receipt)
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
                            return Ok(receipt);
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
            other => {
                let disposition = match other {
                    ActionSend::Accepted => "accepted_without_durable_receipt",
                    ActionSend::Busy => "busy",
                    ActionSend::Gone => "gone",
                    ActionSend::Rejected => "rejected",
                    ActionSend::Finished(_) | ActionSend::Owned(_) => "unexpected",
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
