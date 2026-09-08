//! WS07-S8: the world task publishes a clean exit, and only a clean exit.
//!
//! [`WorldExitStatus`] is the signal the shutdown coordinator's final BGSAVE
//! hangs on. Every proof here drives the **real** world loop from
//! [`world_task::spawn_persist_with_gateway`] — the panic test really panics a
//! command handler rather than simulating one, because the whole point of the
//! flag is that no `catch`, no `Drop`, and no post-loop tail runs on that path.
//!
//! [`WorldExitStatus`]: mush_server::persist::WorldExitStatus

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{
    ArgShape, CmdError, CommandSpec, ControlMsg, Ctx, Engine, GOD, GameModule, Inbound, LIMBO,
    ModError, ObjKind, Registrar,
};
use mush_ipc::{DetachCause, SendTarget};
use mush_proto::OutputEvent;
use mush_server::gateway_sink::GatewaySink;
use mush_server::quarantine::QuarantineSpool;
use mush_server::staff::StaffQueue;
use mush_server::world_task::{
    self, Gateway, PersistHandles, ServerDeps, WorldOutcome, spawn_persist_with_gateway,
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
/// keeps its pending commit indefinitely. That is the state the bounded
/// shutdown grace exists for, and the only way to reach it deterministically.
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

/// Two commands: one that only mutates, and one that mutates and then panics
/// with the mutation still in RAM — the shape of the fault the flag exists to
/// notice.
struct ExitProbeModule;

impl GameModule for ExitProbeModule {
    fn name(&self) -> &'static str {
        "exit_probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "exit-mutate".into(),
                shape: ArgShape::None,
                help: "test-only world mutation".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.w
                    .create(ObjKind::Thing, "exit-probe-orb", ctx.actor, Some(LIMBO))
                    .map_err(|error| CmdError::msg(error.to_string()))?;
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "exit-panic".into(),
                shape: ArgShape::None,
                help: "test-only mutate-then-panic".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.w
                    .create(ObjKind::Thing, "exit-probe-doomed", ctx.actor, Some(LIMBO))
                    .map_err(|error| CmdError::msg(error.to_string()))?;
                panic!("exit-panic: the world task unwinds with the mutation still in RAM");
            }),
        )?;
        Ok(())
    }
}

fn boot(store: Arc<dyn ObjectStore>) -> PersistHandles {
    let engine = Engine::with_modules(
        mush_core::World::new_minimal(),
        world_task::base_registry(),
        vec![Box::new(ExitProbeModule)],
    )
    .expect("exit probe module composes");
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

async fn cmd(handle: &world_task::WorldHandle, line: &str) {
    assert!(
        handle
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: line.to_string(),
            })
            .await,
        "world accepted {line}"
    );
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// The fault the gate exists for: a command handler mutates the world and then
/// panics. Since WS01-S4 the dispatch boundary catches that panic, so the join
/// resolves with [`WorldOutcome::Faulted`] instead of a `JoinError` — but the
/// firewall breaks out *below* the clean-exit publication, so the flag still
/// reads false. `mushd`'s shutdown tail would otherwise see a perfectly drained
/// writer and snapshot the half-world.
#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_world_task_is_not_a_clean_exit() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let handles = boot(store);
    let world_exit = handles.world_exit.clone();
    assert!(
        !world_exit.is_clean(),
        "not clean before the task even runs"
    );

    cmd(&handles.handle, "exit-panic").await;

    let joined = handles.world_join.await.expect("the firewall caught it");
    assert!(
        matches!(joined, WorldOutcome::Faulted(_)),
        "the handler must really panic the world task, not simulate it: {joined:?}"
    );
    assert!(
        !world_exit.is_clean(),
        "a panicked world task must never report a clean exit"
    );
}

/// The one path that may snapshot: a stepped `ControlMsg::Shutdown` whose
/// flush is acknowledged, leaving no pending commit at the foot of the loop.
#[tokio::test(flavor = "multi_thread")]
async fn a_clean_shutdown_is_clean() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let handles = boot(store);
    let world_exit = handles.world_exit.clone();

    cmd(&handles.handle, "exit-mutate").await;
    assert!(handles.handle.barrier().await, "the mutation is acked");
    assert!(
        handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await,
        "world accepted the shutdown control"
    );
    handles.world_join.await.expect("world task shut down");
    assert!(
        world_exit.is_clean(),
        "an acknowledged shutdown is the clean exit"
    );
}

/// The bounded shutdown grace is a give-up, not a completion: the inbound
/// channel closed with a commit still outstanding, the grace ran out, and the
/// loop exited with work whose durable fate it never learned. That is not a
/// clean exit, and the flag must say so even though nothing panicked.
#[tokio::test(start_paused = true)]
async fn a_shutdown_through_the_bounded_grace_is_not_clean() {
    let holding = HoldingStore::new();
    let store: Arc<dyn ObjectStore> = Arc::clone(&holding) as Arc<dyn ObjectStore>;
    let handles = boot(store);
    let world_exit = handles.world_exit.clone();

    // Settle the boot batch first, so the pending commit below can only be the
    // held one.
    assert!(handles.handle.barrier().await, "boot settles");

    holding.hold();
    cmd(&handles.handle, "exit-mutate").await;
    // Let the loop stage the write and take custody of the pending commit.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Dropping the last handle closes the inbound channel: the loop sees the
    // close with a commit pending, waits out its bounded grace, and gives up.
    let closed_at = tokio::time::Instant::now();
    drop(handles.handle);
    handles
        .world_join
        .await
        .expect("the world task exits through the grace without panicking");
    // Virtual time proves WHICH exit ran: a channel close with nothing pending
    // breaks immediately, while the grace costs the loop a 15 s window (less
    // the virtual time already spent above, hence the slack).
    assert!(
        closed_at.elapsed() >= Duration::from_secs(14),
        "the loop must have exited through the bounded grace, not the immediate \
         channel-closed break"
    );
    assert!(
        !world_exit.is_clean(),
        "an exit through the bounded shutdown grace is not a clean exit"
    );
}
