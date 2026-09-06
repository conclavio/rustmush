//! K8.2: real process death on both sides of authoritative IPC ownership.
//!
//! The parent runs the production gateway supervisor and a real SSH client.
//! Its `ServerLauncher` starts this test binary as a separate server process;
//! the worker calls `mush_server::serve` against disposable Redis. Debug-only
//! one-shot hooks stop the writer at an exact durable boundary, then abort the
//! server. The gateway stays alive, respawns the server, and either replays its
//! still-owned request or exposes the already-owned transaction through
//! `action/status`.

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use fred::interfaces::KeysInterface as _;
use futures::future::BoxFuture;
use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_config::metrics::Metrics;
use mush_core::{
    AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey, AuthoritativeSemanticEvent,
    CanonicalHash, CanonicalId, CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, GOD, IdempotencyKey, IdempotencyScope, Ledger, OwnerId,
    PreparedCausalRef, PrincipalId, ReplayWrite, RequestUid, RulesValue, ScopedIdempotencyKey,
};
use mush_gateway::{GatewayOptions, LaunchedServer, RestartTrigger, ServerLauncher, ServerSource};
use mush_ipc::IpcEndpoint;
use mush_server::persist::{PersistenceIntegrityLatch, StoreJob};
use mush_server::quarantine::QuarantineSpool;
use mush_store::{
    AccountRec, AuthoritativeStore, BundleStatus, CharacterRef, InMemoryStore, ObjectStore,
    RedisStore,
};
use russh::ChannelMsg;
use tokio::sync::oneshot;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
const WORKER_ENV: &str = "RUSTMUSH_K8_SERVER_WORKER";
const CONFIG_ENV: &str = "RUSTMUSH_K8_SERVER_CONFIG";
const STOP_ENV: &str = "RUSTMUSH_K8_SERVER_STOP";
const CUT_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT";
const CUT_MARKER_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT_MARKER";
const CUT_RELEASE_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT_RELEASE";
const DIRECT_WORKER_ENV: &str = "RUSTMUSH_K8_DIRECT_WORKER";
const DIRECT_MODE_ENV: &str = "RUSTMUSH_K8_DIRECT_MODE";
const DIRECT_REDIS_ENV: &str = "RUSTMUSH_K8_DIRECT_REDIS";
const DIRECT_SPOOL_ENV: &str = "RUSTMUSH_K8_DIRECT_SPOOL";

const LEDGER_MODE: &str = "ledger_rollover";
const QUARANTINE_MODE: &str = "quarantine_interrupt";

static PROCESS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();
}

fn scratch_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mush-k8-process-{tag}-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn redis_url_with_db(base: &str, db: u32) -> String {
    let (scheme, rest) = base.split_once("://").unwrap_or(("redis", base));
    let authority = rest.split(['/', '?']).next().unwrap_or(rest);
    format!("{scheme}://{authority}/{db}")
}

fn direct_bundle(mode: &str) -> AuthoritativeBundle {
    let owner = OwnerId::new("k8_process").unwrap();
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new(mode).unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let outcome = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").unwrap(),
                DefinitionId::new("committed").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let identity = if mode == LEDGER_MODE { 0xc1 } else { 0xd1 };
    let request = RequestUid::from_bytes([identity; 16]);
    let token = CommitToken::from_bytes([identity.wrapping_add(1); 16]);
    let principal = PrincipalId::Object { id: GOD };
    let writes = match mode {
        LEDGER_MODE => {
            let mut ledger = Ledger::with_chunk_target(0);
            let prepared = ledger
                .prepare_append("rollover", serde_json::json!({"amount": 7}))
                .unwrap();
            let replay = prepared.authoritative_replay_write(owner.clone()).unwrap();
            ledger.apply_prepared(prepared).unwrap();
            assert!(
                ledger.open_entries("rollover").is_empty(),
                "target-zero append must roll to a fresh empty tail"
            );
            vec![replay]
        }
        QUARANTINE_MODE => vec![ReplayWrite::ModuleJsonSet {
            namespace: owner.clone(),
            key: AuthoritativeKey::parse_canonical("action-state/v1/process_poison/1").unwrap(),
            value: serde_json::json!({"value": 1}),
        }],
        _ => panic!("unknown direct worker mode {mode}"),
    };
    let outcome_value = RulesValue::new(outcome.clone(), serde_json::json!({"ok": true})).unwrap();
    let cause = PreparedCausalRef::new(request, None, 0).unwrap();
    let semantic_event = AuthoritativeSemanticEvent::root(
        cause,
        &action,
        outcome_value.clone(),
        vec![principal.clone()],
    )
    .unwrap();
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("process_cut").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([identity.wrapping_add(2); 32]),
        request_hash: CanonicalHash::from_bytes([identity.wrapping_add(3); 32]),
        plan_hash: CanonicalHash::from_bytes([identity.wrapping_add(4); 32]),
        action: action.clone(),
        provider: owner,
        cause,
        definitions: vec![action, outcome],
        writes,
        outcome: outcome_value,
        resolution: None,
        semantic_events: vec![semantic_event],
        eligible_witnesses: vec![principal],
        durable_outbox: vec![],
    })
    .unwrap()
}

fn reserve_loopback_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

fn write_server_config(root: &Path, redis_url: &str, endpoint: SocketAddr) -> PathBuf {
    let data = root.join("server-data");
    std::fs::create_dir_all(&data).unwrap();
    let data = data.display().to_string().replace('\\', "/");
    let path = root.join("server.toml");
    std::fs::write(
        &path,
        format!(
            "[server]\nname = \"K8 process worker\"\ndata_dir = \"{data}\"\n\
             [store]\nstore = \"redis\"\nredis_url = \"{redis_url}\"\nreceipt_policy = \"applied\"\n\
             [ipc]\nlisten = \"{endpoint}\"\n\
             [rulesets]\nenabled = []\n"
        ),
    )
    .unwrap();
    path
}

async fn wait_for_file(path: &Path) {
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    while !path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn seeded_gateway_store() -> Arc<dyn ObjectStore> {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_owned(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_owned(),
            }],
            ssh_keys: Vec::new(),
            discord: None,
        })
        .await
        .unwrap();
    store
}

struct ProcessServerLauncher {
    config: PathBuf,
    endpoint: IpcEndpoint,
    cut: &'static str,
    marker: PathBuf,
    release: PathBuf,
    stop: PathBuf,
}

struct StopGuard(PathBuf);

impl Drop for StopGuard {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, b"stop\n");
    }
}

impl ServerLauncher for ProcessServerLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        if self.stop.exists() {
            return Box::pin(async { Err("K8 process worker stopped".to_owned()) });
        }
        let current_exe = std::env::current_exe();
        let config = self.config.clone();
        let endpoint = self.endpoint.clone();
        let cut = self.cut;
        let marker = self.marker.clone();
        let release = self.release.clone();
        let stop = self.stop.clone();
        Box::pin(async move {
            let executable = current_exe.map_err(|error| error.to_string())?;
            let mut child = tokio::process::Command::new(executable)
                .arg("--exact")
                .arg("authoritative_server_process_worker")
                .arg("--nocapture")
                .arg("--test-threads=1")
                .env(WORKER_ENV, "1")
                .env(CONFIG_ENV, &config)
                .env(STOP_ENV, &stop)
                .env(CUT_ENV, cut)
                .env(CUT_MARKER_ENV, &marker)
                .env(CUT_RELEASE_ENV, &release)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .spawn()
                .map_err(|error| format!("spawning K8 server worker: {error}"))?;
            // This gated proof may reach Redis through a WAN SSH tunnel. Let
            // the worker finish module hydration before starting the
            // production supervisor's intentionally short local-child connect
            // window; the crash/replay path below remains fully process split.
            tokio::time::sleep(Duration::from_secs(15)).await;
            Ok(LaunchedServer {
                endpoint,
                exited: Box::pin(async move {
                    let _ = child.wait().await;
                }),
            })
        })
    }
}

fn gateway_config(root: &Path) -> Config {
    let mut config = Config::default();
    config.store.store = "memory".to_owned();
    config.server.data_dir = root.join("gateway-data");
    config.ssh.listen = "127.0.0.1:0".to_owned();
    config.web = toml::Value::Table(toml::Table::from_iter([(
        "listen".to_owned(),
        toml::Value::String("127.0.0.1:0".to_owned()),
    )]));
    config.ipc.listen = "127.0.0.1:0".to_owned();
    config.rulesets.enabled = Vec::new();
    config
}

async fn start_gateway(
    root: &Path,
    launcher: ProcessServerLauncher,
) -> (SocketAddr, RestartTrigger, std::thread::JoinHandle<()>) {
    let store = seeded_gateway_store().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let (restart_tx, restart_rx) = oneshot::channel();
    let config = gateway_config(root);
    let join = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(mush_gateway::serve(
                config,
                GatewayOptions {
                    source: ServerSource::Restartable(Box::new(launcher)),
                    store: Some(store),
                    ssh_ready: Some(ssh_tx),
                    web_ready: None,
                    restart_ready: Some(restart_tx),
                },
            ))
            .unwrap();
    });
    let address = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway SSH bind timed out")
        .expect("gateway dropped SSH address");
    let restart = tokio::time::timeout(Duration::from_secs(30), restart_rx)
        .await
        .expect("gateway restart trigger timed out")
        .expect("gateway dropped restart trigger");
    (address, restart, join)
}

struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    fn mark(&self) -> usize {
        self.seen.len()
    }

    async fn send_line(&mut self, line: &str) {
        self.channel
            .data(format!("{line}\r").as_bytes())
            .await
            .unwrap();
    }

    async fn expect_after(&mut self, mark: usize, needle: &str) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while !strip_ansi(&self.seen[mark..]).contains(needle) {
            let message = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!("timeout waiting for {needle:?}; seen: {:?}", self.seen)
                });
            match message {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for {needle:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let output = strip_ansi(&self.seen[mark..]);
            if let Some((index, _)) = needles
                .iter()
                .enumerate()
                .find(|(_, needle)| output.contains(**needle))
            {
                return index;
            }
            let message = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!("timeout waiting for {needles:?}; seen: {:?}", self.seen)
                });
            match message {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for {needles:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut characters = input.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' && characters.peek() == Some(&'[') {
            characters.next();
            for final_byte in characters.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&final_byte) {
                    break;
                }
            }
        } else if character != '\u{1b}' {
            output.push(character);
        }
    }
    output
}

fn transaction_after(output: &str, mark: usize) -> String {
    let output = strip_ansi(&output[mark..]);
    let at = output
        .rfind("Transaction:")
        .unwrap_or_else(|| panic!("missing transaction reference in {output:?}"));
    output[at + "Transaction:".len()..]
        .trim_start()
        .chars()
        .take_while(|character| !character.is_whitespace())
        .collect()
}

async fn connect(address: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, address, AcceptAll)
        .await
        .unwrap();
    assert!(handle.authenticate_none("play").await.unwrap().success());
    let channel = handle.channel_open_session().await.unwrap();
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .unwrap();
    channel.request_shell(true).await.unwrap();
    (
        handle,
        Term {
            channel,
            seen: String::new(),
        },
    )
}

async fn enter_when_boundary_is_ready(term: &mut Term) {
    for _ in 0..30 {
        let mark = term.mark();
        term.send_line(&format!("connect boss {GM_PASSWORD}")).await;
        match term
            .expect_one_of_after(
                mark,
                &[
                    "Connected as God",
                    "Connection failed:",
                    "You're doing that too fast.",
                ],
            )
            .await
        {
            0 => {
                term.expect_after(mark, "Limbo #0").await;
                return;
            }
            1 | 2 => tokio::time::sleep(Duration::from_secs(2)).await,
            _ => unreachable!(),
        }
    }
    panic!("initial server boundary did not become ready");
}

async fn wait_for_stop(path: PathBuf) {
    while !path.exists() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Separate-process server entry point selected by `ProcessServerLauncher`.
#[test]
fn authoritative_server_process_worker() {
    if std::env::var_os(WORKER_ENV).is_none() {
        return;
    }
    init_tracing();
    let config = Config::load(Path::new(
        &std::env::var_os(CONFIG_ENV).expect("worker config path"),
    ))
    .unwrap();
    let stop = PathBuf::from(std::env::var_os(STOP_ENV).expect("worker stop path"));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async move {
        tokio::select! {
            result = mush_server::serve(config, None) => result,
            () = wait_for_stop(stop) => Ok(()),
        }
    });
    if let Err(error) = result {
        panic!("K8 server worker failed: {error}");
    }
}

/// Separate process that drives one exact bundle through the production store
/// writer without involving a presentation path. The parent uses this for
/// crash boundaries whose proof is entirely persistence-local: a sealed
/// Ledger rollover and atomic quarantine publication.
#[test]
fn authoritative_direct_process_worker() {
    if std::env::var_os(DIRECT_WORKER_ENV).is_none() {
        return;
    }
    init_tracing();
    let mode = std::env::var(DIRECT_MODE_ENV).expect("direct worker mode");
    let redis_url = std::env::var(DIRECT_REDIS_ENV).expect("direct worker Redis URL");
    let spool_path = PathBuf::from(
        std::env::var_os(DIRECT_SPOOL_ENV).expect("direct worker quarantine spool path"),
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let store: Arc<dyn ObjectStore> = Arc::new(RedisStore::connect(&redis_url).await.unwrap());
        let spool = QuarantineSpool::open(spool_path).unwrap();
        let (signals, _signal_rx) = mush_server::world_task::writer_signal_channel();
        let (writer, _join) = mush_server::persist::spawn_store_writer(
            store,
            signals,
            Arc::new(Metrics::default()),
            Arc::new(AtomicU64::new(0)),
            spool,
            PersistenceIntegrityLatch::new(),
        );
        let (ownership, owned) = oneshot::channel();
        let (notify, done) = oneshot::channel();
        writer
            .send(StoreJob::Authoritative {
                bundle: Box::new(direct_bundle(&mode)),
                ownership,
                notify,
            })
            .await
            .unwrap();
        let _ = owned.await;
        let result = done.await;
        panic!("direct process cut did not terminate the worker: {result:?}");
    });
}

async fn run_direct_process_cut(
    redis_url: &str,
    spool_path: &Path,
    mode: &str,
    cut: &str,
    marker: &Path,
) {
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("authoritative_direct_process_worker")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(DIRECT_WORKER_ENV, "1")
        .env(DIRECT_MODE_ENV, mode)
        .env(DIRECT_REDIS_ENV, redis_url)
        .env(DIRECT_SPOOL_ENV, spool_path)
        .env(CUT_ENV, cut)
        .env(CUT_MARKER_ENV, marker)
        .env_remove(CUT_RELEASE_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    wait_for_file(marker).await;
    let status = tokio::time::timeout(STEP_TIMEOUT, child.wait())
        .await
        .expect("direct process worker did not exit")
        .expect("waiting for direct process worker");
    assert!(!status.success(), "process cut must terminate the worker");
}

async fn run_process_cut(base_redis: &str, db: u32, cut: &'static str, owned_cut: bool) {
    let root = scratch_dir(cut);
    let redis_url = redis_url_with_db(base_redis, db);
    let admin = RedisStore::connect(&redis_url).await.unwrap();
    admin.wipe_namespaces().await.unwrap();

    let endpoint = reserve_loopback_addr();
    let config = write_server_config(&root, &redis_url, endpoint);
    let marker = root.join("cut.ready");
    let release = root.join("cut.release");
    let stop = root.join("server.stop");
    let _stop_guard = StopGuard(stop.clone());
    let launcher = ProcessServerLauncher {
        config,
        endpoint: IpcEndpoint::Tcp(endpoint),
        cut,
        marker: marker.clone(),
        release: release.clone(),
        stop: stop.clone(),
    };
    let (address, _restart, _gateway) = start_gateway(&root, launcher).await;
    let (_client, mut term) = connect(address).await;
    // SSH binds independently of the server boundary. Remote Redis boot and
    // module hydration can take several seconds, so avoid turning that normal
    // initial gap into an authentication-rate-limit test.
    tokio::time::sleep(Duration::from_secs(5)).await;
    enter_when_boundary_is_ready(&mut term).await;
    term.send_line("+actions").await;
    term.expect_after(0, "brace — Brace").await;

    let action_mark = term.mark();
    term.send_line("+act brace").await;
    wait_for_file(&marker).await;
    if owned_cut {
        term.expect_after(action_mark, "Action accepted. Transaction:")
            .await;
    }
    std::fs::write(&release, b"crash\n").unwrap();

    term.expect_after(action_mark, "restarting").await;
    term.expect_after(action_mark, "Reconnected").await;
    if !owned_cut {
        // The gateway still owned the request, so replay joins the boot-healed
        // receipt and renders the terminal result directly.
        term.expect_after(action_mark, "Brace committed. Transaction:")
            .await;
    }
    let transaction = transaction_after(&term.seen, action_mark);
    let status_mark = term.mark();
    term.send_line(&format!("action/status {transaction}"))
        .await;
    term.expect_after(status_mark, "Brace committed.").await;

    let state = admin
        .module_json_get("tactics", "action-state/v1/stance/1")
        .await
        .unwrap()
        .expect("brace state persisted");
    assert_eq!(state["value"]["braced"], true);
    assert_eq!(
        state["value"]["revision"], 1,
        "gateway replay must join rather than execute the action twice"
    );

    std::fs::write(&stop, b"stop\n").unwrap();
    term.expect_after(term.mark(), "could not be restarted")
        .await;
    admin.wipe_namespaces().await.unwrap();
    admin.close().await.ok();
    let _ = std::fs::remove_dir_all(root);
}

async fn run_ledger_rollover_process_cut(base_redis: &str) {
    let root = scratch_dir(LEDGER_MODE);
    let redis_url = redis_url_with_db(base_redis, 0);
    let admin = RedisStore::connect(&redis_url).await.unwrap();
    admin.wipe_namespaces().await.unwrap();
    mush_server::persist::load_or_seed_world(&admin)
        .await
        .unwrap();
    let spool_path = root.join("quarantine");
    let marker = root.join("cut.ready");
    run_direct_process_cut(
        &redis_url,
        &spool_path,
        LEDGER_MODE,
        "after_finalize_before_checkpoint",
        &marker,
    )
    .await;

    let bundle = direct_bundle(LEDGER_MODE);
    let report = mush_server::persist::reconcile_authoritative_for_writable_boot(
        admin.authoritative_store().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(report.already_finalized, 1);
    assert_eq!(report.checkpointed, 1);
    assert!(matches!(
        admin
            .authoritative_status(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap(),
        BundleStatus::FinalizedExact(receipt) if receipt.matches_bundle(&bundle)
    ));
    let chunk = admin
        .module_json_get("k8_process", "log:rollover:0")
        .await
        .unwrap()
        .expect("sealed rollover chunk survived process death");
    assert_eq!(chunk["sealed"], true);
    assert_eq!(chunk["chunk"], 0);
    assert_eq!(chunk["entries"].as_array().map(Vec::len), Some(1));

    admin.wipe_namespaces().await.unwrap();
    admin.close().await.ok();
    let _ = std::fs::remove_dir_all(root);
}

async fn run_quarantine_process_cut(base_redis: &str) {
    let root = scratch_dir(QUARANTINE_MODE);
    let redis_url = redis_url_with_db(base_redis, 0);
    let admin = RedisStore::connect(&redis_url).await.unwrap();
    admin.wipe_namespaces().await.unwrap();
    mush_server::persist::load_or_seed_world(&admin)
        .await
        .unwrap();
    let poison_key = "mod:k8_process:action-state/v1/process_poison/1";
    let _: () = admin
        .client()
        .set(poison_key, "wrong type", None, None, false)
        .await
        .unwrap();
    let spool_path = root.join("quarantine");
    let marker = root.join("cut.ready");
    run_direct_process_cut(
        &redis_url,
        &spool_path,
        QUARANTINE_MODE,
        "quarantine_after_temp_sync",
        &marker,
    )
    .await;

    let active_before_recovery: Vec<_> = std::fs::read_dir(&spool_path)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    assert_eq!(
        active_before_recovery
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
            .count(),
        1,
        "the killed writer leaves one exact, fsynced temp record"
    );
    assert_eq!(
        active_before_recovery
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .count(),
        0,
        "the cut occurs before atomic final-name publication"
    );

    let deleted: i64 = admin.client().del(poison_key).await.unwrap();
    assert_eq!(deleted, 1);
    let mut spool = QuarantineSpool::open(&spool_path).unwrap();
    assert_eq!(spool.opened_state().unresolved_records, 1);
    let mut config = Config::default();
    config.server.data_dir = root.clone();
    config.store.store = "redis".to_owned();
    config.store.redis_url = redis_url.clone();
    mush_server::prepare_quarantine_for_writable_boot(&config, &mut spool, Some(&admin))
        .await
        .unwrap();
    assert_eq!(spool.opened_state().unresolved_records, 0);
    let bundle = direct_bundle(QUARANTINE_MODE);
    let report = mush_server::persist::reconcile_authoritative_for_writable_boot(
        admin.authoritative_store().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(report.already_finalized, 1);
    assert_eq!(report.checkpointed, 1);
    assert_eq!(
        admin
            .module_json_get("k8_process", "action-state/v1/process_poison/1")
            .await
            .unwrap(),
        Some(serde_json::json!({"value": 1}))
    );
    assert!(matches!(
        admin
            .authoritative_status(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap(),
        BundleStatus::FinalizedExact(receipt) if receipt.matches_bundle(&bundle)
    ));

    drop(spool);
    admin.wipe_namespaces().await.unwrap();
    admin.close().await.ok();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_gateway_recovers_across_authoritative_process_cuts() {
    let Some(base) = std::env::var("REDIS_URL")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping K8.2 process-kill matrix: REDIS_URL not set");
        return;
    };
    init_tracing();
    let _serial = PROCESS_LOCK.lock().await;
    // RediSearch is database-zero only. The dedicated test instance is wiped
    // between these serialized cuts rather than pretending logical DBs isolate
    // the complete production schema.
    run_process_cut(&base, 0, "after_intent_before_ownership", false).await;
    run_process_cut(&base, 0, "after_ownership_before_materialization", true).await;
    run_process_cut(&base, 0, "after_finalize_before_checkpoint", true).await;
    run_process_cut(&base, 0, "after_checkpoint_before_final_ack", true).await;
    run_ledger_rollover_process_cut(&base).await;
    run_quarantine_process_cut(&base).await;
}
