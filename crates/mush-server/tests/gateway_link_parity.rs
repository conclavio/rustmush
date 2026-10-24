//! WS04-S6: both deployments answer a `ServiceReq` from ONE dispatch table.
//!
//! The forked server's boundary reader and the all-in-one binary used to carry
//! two near-identical fifty-line matches over [`ServiceReq`] — one in
//! `lib.rs`, one in the in-process [`GatewayLink`] impl. Two exhaustive matches
//! over the same enum is a drift surface with no compiler behind it: a variant
//! answered `Unavailable` on one link and `Busy` on the other — or answered on
//! one link and not the other — would compile and ship, and which of the two
//! deployments a player hit would decide what their client was told. Since
//! this step there is one table ([`mush_server::handle_request`]) and the
//! in-process link delegates to it.
//!
//! The proofs drive a **real** world task in the two states where the
//! dispositions actually differ — gone, and holding a commit — and assert the
//! two links answer *equal* values for every variant. The sample vector is
//! checked against an exhaustive match, so an eleventh `ServiceReq` variant fails
//! to compile here rather than being silently left out of the sweep.
//!
//! [`GatewayLink`]: mush_ipc::GatewayLink

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{
    AccountId, ArgShape, CmdError, CommandSpec, ControlMsg, Ctx, Engine, GOD, GameModule, Inbound,
    LIMBO, ModError, ObjKind, PerceivedRef, Registrar,
};
use mush_ipc::{
    CreatePlayerError, DetachCause, DiscordInteraction, GatewayLink, RelaySpeak, SendTarget,
    ServiceReq, ServiceResp,
};
use mush_proto::{ActionStatusId, NarrativeKind, OutputEvent};
use mush_server::gateway_sink::GatewaySink;
use mush_server::quarantine::QuarantineSpool;
use mush_server::staff::StaffQueue;
use mush_server::world_task::{
    self, Gateway, PersistHandles, ServerDeps, WorldHandle, WorldOutcome, WorldReadError,
    spawn_persist_with_gateway,
};
use mush_store::{
    AccountRec, BanRec, DiscordBanRec, DiscordCodeRecord, DiscordLinkRec, InMemoryStore,
    ObjectStore, SshKeyRec, StoreError, TokenRecord, WorldSnapshot, WriteBatch, WriteReceipt,
};

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

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

/// An [`ObjectStore`] whose `write` can be made to **never answer**: the
/// batch reaches the writer, the writer awaits the store, and the world loop
/// keeps its pending commit indefinitely. That is `WorldMode::CommitPending`,
/// and holding a real store is the only way to reach it deterministically —
/// which is what this file needs, because `CommitPending` is where
/// `ServiceResp::Busy` actually originates: the world loop's refusal fan-out
/// turns `RefusalReason::CommitPending` into `WorldReadError::Busy`.
///
/// Lifted from `tests/world_exit_status.rs`; each integration-test binary owns
/// its own store double.
struct HoldingStore {
    inner: InMemoryStore,
    holding: AtomicBool,
}

impl HoldingStore {
    fn new() -> Arc<HoldingStore> {
        Arc::new(HoldingStore {
            inner: InMemoryStore::new(),
            holding: AtomicBool::new(false),
        })
    }

    fn hold(&self) {
        self.holding.store(true, Ordering::SeqCst);
    }
}

impl ObjectStore for HoldingStore {
    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        Box::pin(async move {
            if self.holding.load(Ordering::SeqCst) {
                // Not an error and not a timeout: the store simply never
                // answers, exactly like a Redis that stopped replying.
                std::future::pending::<()>().await;
            }
            self.inner.write(batch).await
        })
    }

    fn authoritative_store(&self) -> Option<&dyn mush_store::AuthoritativeStore> {
        self.inner.authoritative_store()
    }

    fn bootstrap_writable(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.bootstrap_writable()
    }

    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        self.inner.load_world()
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
        id: mush_core::ObjId,
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

/// One command that only mutates, so a proof can put a real write in flight
/// and leave the loop holding its pending commit.
struct ParityProbeModule;

impl GameModule for ParityProbeModule {
    fn name(&self) -> &'static str {
        "gateway_link_parity"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "parity-mutate".into(),
                shape: ArgShape::None,
                help: "test-only world mutation".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.w
                    .create(ObjKind::Thing, "parity-probe-orb", ctx.actor, Some(LIMBO))
                    .map_err(|error| CmdError::msg(error.to_string()))?;
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn boot(store: Arc<dyn ObjectStore>) -> PersistHandles {
    let engine = Engine::with_modules(
        mush_core::World::new_minimal(),
        world_task::base_registry(),
        vec![Box::new(ParityProbeModule)],
    )
    .expect("parity probe module composes");
    spawn_persist_with_gateway(
        engine,
        ServerDeps {
            store,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        |_| -> Gateway { Arc::new(BlackholeGateway) },
    )
}

// ---------------------------------------------------------------------------
// The sample vector, and the compile-time coverage behind it
// ---------------------------------------------------------------------------

/// How many variants [`ServiceReq`] has today. Restated rather than derived:
/// the enum lives in `mush-ipc` and exposes no count, and this number is the
/// second half of the coverage gate — [`variant_name`] refuses to COMPILE when
/// a variant is added, and this refuses to PASS when an arm is added there
/// without a sample below.
const SERVICE_REQ_VARIANTS: usize = 10;

/// Name each variant through an exhaustive match.
///
/// This is the durable half of the proof: adding an eleventh `ServiceReq` arm in
/// `mush-ipc` makes this match non-exhaustive, so this file stops compiling
/// until the new variant is named — and once named it must also be sampled in
/// [`every_service_req`], or the count assertion there fails.
fn variant_name(req: &ServiceReq) -> &'static str {
    match req {
        ServiceReq::CreatePlayer { .. } => "CreatePlayer",
        ServiceReq::UnlinkPlayer { .. } => "UnlinkPlayer",
        ServiceReq::NameFree { .. } => "NameFree",
        ServiceReq::InInputContext { .. } => "InInputContext",
        ServiceReq::ConnectionReady { .. } => "ConnectionReady",
        ServiceReq::LegalActions { .. } => "LegalActions",
        ServiceReq::ActionStatus { .. } => "ActionStatus",
        ServiceReq::Barrier => "Barrier",
        ServiceReq::DiscordInteraction(_) => "DiscordInteraction",
        ServiceReq::RelaySpeak(_) => "RelaySpeak",
    }
}

/// One sample of every [`ServiceReq`] variant. The payloads are deliberately
/// uninteresting: both sweeps run against a world that refuses every read, so
/// what a live world would have answered never enters the comparison.
fn every_service_req() -> Vec<ServiceReq> {
    let samples = vec![
        ServiceReq::CreatePlayer {
            name: "parity-probe".to_string(),
            account: None,
        },
        ServiceReq::UnlinkPlayer { character: GOD },
        ServiceReq::NameFree {
            name: "parity-probe".to_string(),
        },
        ServiceReq::InInputContext { actor: GOD },
        ServiceReq::ConnectionReady {
            conn: 7,
            actor: GOD,
        },
        ServiceReq::LegalActions {
            conn: 7,
            actor: GOD,
            subject: PerceivedRef::SelfActor,
        },
        ServiceReq::ActionStatus {
            conn: 7,
            actor: GOD,
            status: ActionStatusId::new(
                "k5:parity:probe:0000000000001234:01010101010101010101010101010101",
            )
            .expect("sample action status id"),
        },
        ServiceReq::Barrier,
        ServiceReq::DiscordInteraction(DiscordInteraction {
            user_id: 7,
            account: None,
            actor: None,
            custom_id: "d:parity:probe".to_string(),
            values: Vec::new(),
        }),
        ServiceReq::RelaySpeak(RelaySpeak {
            message_id: None,
            dm_conn: None,
            user_id: 7,
            account: AccountId::mint("parity-probe", 0).expect("sample account id"),
            actor: GOD,
            channel: "public".to_string(),
            kind: NarrativeKind::Say,
            text: "probe".to_string(),
        }),
    ];

    let mut names: Vec<&'static str> = samples.iter().map(variant_name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        samples.len(),
        "one sample per variant, and no variant sampled twice"
    );
    assert_eq!(
        samples.len(),
        SERVICE_REQ_VARIANTS,
        "every ServiceReq variant needs a sample here; `variant_name` already \
         refuses to compile without one"
    );
    samples
}

/// How long a proof waits for either link before calling a request unanswered.
///
/// An ordering barrier queued behind a pending commit is answered only when
/// that commit acknowledges, which under a held store is never. Both links owe
/// the same silence there, so the sweep records "no answer" and compares it
/// rather than hanging on it.
const ANSWER_WAIT: Duration = Duration::from_secs(2);

/// Drive one sample through BOTH links and return what each answered.
///
/// `None` means "did not answer inside [`ANSWER_WAIT`]". The in-process link's
/// own `Option` is never `None` — it wraps every answer in `Some` — so the two
/// cannot be confused.
async fn both_links(
    world: &WorldHandle,
    req: ServiceReq,
) -> (Option<ServiceResp>, Option<ServiceResp>) {
    let table = tokio::time::timeout(ANSWER_WAIT, mush_server::handle_request(world, req.clone()))
        .await
        .ok();
    let link = tokio::time::timeout(ANSWER_WAIT, GatewayLink::request(world, req))
        .await
        .ok()
        .flatten();
    (table, link)
}

/// A world handle whose task has really shut down and been JOINED: the
/// receiver is gone, so every request from here on is refused at the channel.
async fn dead_world() -> WorldHandle {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let handles = boot(store);
    let handle = handles.handle.clone();
    assert!(
        handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await,
        "the live world accepts the shutdown control"
    );
    assert_eq!(
        handles.world_join.await.expect("world task shut down"),
        WorldOutcome::Clean,
        "an operator shutdown is clean"
    );
    drop(handles.handle);
    assert!(
        !handle
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "look".to_string(),
            })
            .await,
        "the world really is unreachable before the sweep runs"
    );
    handle
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// The drift the one table exists to make impossible, in the state where the
/// two matches were most likely to disagree: a world that is gone. Every
/// variant, both links, equal answers.
#[tokio::test(flavor = "multi_thread")]
async fn both_links_answer_identically_on_a_dead_world() {
    let world = dead_world().await;
    for req in every_service_req() {
        let name = variant_name(&req);
        let (table, link) = both_links(&world, req).await;
        assert_eq!(
            table, link,
            "{name}: the boundary reader's table and the in-process link must \
             answer a dead world identically"
        );
        assert!(
            table.is_some(),
            "{name}: a dead world answers at the channel; nothing may hang"
        );
    }
}

/// The same sweep in the state `ServiceResp::Busy` actually comes from: a real
/// commit staged against a store that never acknowledges it, which leaves the
/// loop in `WorldMode::CommitPending` and every world read refused as
/// retryable.
///
/// Note what is NOT done here: the boundary's `ServiceRequestLimiter` also
/// emits `Busy` on overload, but it sits upstream of the shared table in the
/// IPC reader and is no part of what these two links share.
#[tokio::test(start_paused = true)]
async fn both_links_answer_identically_under_commit_pending() {
    let holding = HoldingStore::new();
    let store: Arc<dyn ObjectStore> = Arc::clone(&holding) as Arc<dyn ObjectStore>;
    let handles = boot(store);

    // Settle the boot batch first, so the commit held below is the only one.
    assert!(handles.handle.barrier().await, "boot settles");

    holding.hold();
    assert!(
        handles
            .handle
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "parity-mutate".to_string(),
            })
            .await,
        "the live world accepts the mutation"
    );

    // Wait for the loop to actually take custody of the pending commit rather
    // than assume one sleep was enough: until it does, reads still answer.
    let mut pending = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if handles.handle.name_free("parity-probe").await == Err(WorldReadError::Busy) {
            pending = true;
            break;
        }
    }
    assert!(
        pending,
        "the held write must leave the world commit-pending"
    );

    for req in every_service_req() {
        let name = variant_name(&req);
        let (table, link) = both_links(&handles.handle, req).await;
        assert_eq!(
            table, link,
            "{name}: the boundary reader's table and the in-process link must \
             answer a commit-pending world identically"
        );
    }
}

/// The dispositions themselves, pinned. Equality alone would still pass if a
/// refactor collapsed every arm into one answer, so this names what each
/// variant owes a caller whose world is gone — in particular that
/// `CreatePlayer` reports `WorldGone` INSIDE its own result rather than the
/// request-level `Unavailable`, which is how a caller tells "the creation
/// definitely did not happen" from "ask again later".
#[tokio::test(flavor = "multi_thread")]
async fn the_disposition_table_is_pinned() {
    let world = dead_world().await;
    for req in every_service_req() {
        let name = variant_name(&req);
        let expected = match name {
            "CreatePlayer" => ServiceResp::CreatePlayer(Err(CreatePlayerError::WorldGone)),
            // The other eight are backed by `WorldReadError` or
            // `LegalActionReadError`, whose gone-world arm is `Unavailable`.
            _ => ServiceResp::Unavailable,
        };
        let (table, link) = both_links(&world, req).await;
        assert_eq!(table, Some(expected.clone()), "{name}: shared table");
        assert_eq!(link, Some(expected), "{name}: in-process link");
    }
}
