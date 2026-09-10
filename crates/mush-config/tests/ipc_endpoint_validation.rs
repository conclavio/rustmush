//! WS08-S13: `Config::ipc_endpoint()` validates the forked gateway<->server
//! boundary endpoint on every load path.
//!
//! The failure mode these prove: a split deployment whose config never wrote an
//! `[ipc]` section inherits `IpcCfg::default().listen` = `"127.0.0.1:6791"`. On
//! an instance whose own Redis lives on 6791 (dev2 did, exactly), the server
//! would try to bind the boundary on top of the store, fail, and crash-loop
//! under the gateway's relaunch guard. The refusal names both keys so an
//! operator can fix it without reading the source; a TCP endpoint that does not
//! collide is accepted but warns loudly, because the boundary is
//! unauthenticated and TCP lets any local process impersonate the gateway.

use std::sync::{Arc, Mutex, MutexGuard};

use mush_config::config::{Config, ConfigError};

/// Every test here runs `ipc_endpoint()`, and one of them captures its log
/// under a thread-scoped subscriber. `tracing` registers a callsite lazily on
/// its first hit and, while exactly one dispatcher is registered, evaluates the
/// callsite's interest against the *registering thread's* default subscriber.
/// A sibling test hitting the warn first on a thread with no subscriber
/// therefore caches the callsite as never-enabled, and the capturing test sees
/// nothing (seen once under a loaded `cargo test --workspace -j 6`). Serialising
/// the tests removes the interleaving: each scoped subscriber's registration
/// rebuilds every callsite's interest before its own test runs.
static SERIAL: Mutex<()> = Mutex::new(());

fn serialized() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A `MakeWriter` that collects formatted log lines into a shared buffer.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

/// Run `f` with a scoped subscriber and return what it logged alongside the
/// result.
fn with_captured_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(capture.clone())
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    (out, capture.text())
}

#[test]
fn an_ipc_port_that_collides_with_the_store_is_refused() {
    let _serial = serialized();
    let mut cfg = Config::default();
    // dev2's own Redis port — and, verbatim, `IpcCfg::default().listen`.
    cfg.store.redis_url = "redis://127.0.0.1:6791".to_string();
    assert_eq!(
        cfg.ipc.listen, "127.0.0.1:6791",
        "this test is only meaningful while the unset [ipc] default is the \
         port it collides with"
    );

    let err = cfg
        .ipc_endpoint()
        .expect_err("the boundary must not be allowed to bind the store's port");
    match &err {
        ConfigError::IpcStorePortCollision { port, .. } => assert_eq!(*port, 6791),
        other => panic!("expected a port-collision refusal, got {other:?}"),
    }
    let text = err.to_string();
    assert!(
        text.contains("[ipc].listen"),
        "must name [ipc].listen: {text}"
    );
    assert!(
        text.contains("[store].redis_url"),
        "must name [store].redis_url: {text}"
    );
    assert!(text.contains("6791"), "must name the shared port: {text}");
}

#[test]
fn a_remote_store_on_the_same_port_number_is_not_a_collision() {
    let _serial = serialized();
    let mut cfg = Config::default();
    // Same port, another box: binding 127.0.0.1:6791 here is fine, and
    // refusing it would be a false refusal on a legitimate config.
    cfg.store.redis_url = "redis://10.0.0.7:6791".to_string();
    cfg.ipc_endpoint()
        .expect("a store on another host does not occupy this box's port");
}

#[test]
fn a_memory_store_never_collides() {
    let _serial = serialized();
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.store.redis_url = "redis://127.0.0.1:6791".to_string();
    cfg.ipc_endpoint()
        .expect("`store = \"memory\"` never contacts redis_url, so nothing binds that port");
}

#[test]
fn a_distinct_tcp_endpoint_is_accepted_with_a_warn() {
    let _serial = serialized();
    let mut cfg = Config::default();
    cfg.store.redis_url = "redis://127.0.0.1:6788".to_string();
    cfg.ipc.listen = "127.0.0.1:6794".to_string();

    let (endpoint, logs) = with_captured_logs(|| cfg.ipc_endpoint());
    endpoint.expect("a free localhost port is a usable (if warned-about) boundary");
    assert!(
        logs.contains("WARN"),
        "a TCP boundary must warn loudly: {logs}"
    );
    assert!(
        logs.contains("impersonate"),
        "the warn must state the impersonation hazard: {logs}"
    );
    assert!(
        logs.contains("Unix socket"),
        "the warn must point at the Unix-socket form: {logs}"
    );
}

#[cfg(unix)]
#[test]
fn a_unix_socket_endpoint_is_accepted_silently() {
    let _serial = serialized();
    let mut cfg = Config::default();
    cfg.store.redis_url = "redis://127.0.0.1:6791".to_string();
    cfg.ipc.listen = "/run/rustmush/dev2.sock".to_string();

    let (endpoint, logs) = with_captured_logs(|| cfg.ipc_endpoint());
    endpoint.expect("a Unix socket path is the prod form and must be accepted");
    assert!(
        !logs.contains("WARN"),
        "the recommended form must not nag: {logs}"
    );
}

/// Parse a shipped deploy config the way `deploy.sh` installs it and run it
/// through the same validation the binaries do.
fn validate_shipped(instance: &str) {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let src = repo.join("deploy").join(instance).join("mushd.toml");
    let text = std::fs::read_to_string(&src)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", src.display()))
        .replace("__EXTERNAL_IP__", "203.0.113.9");

    let dir = std::env::temp_dir().join(format!("mush-cfg-{instance}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mushd.toml");
    std::fs::write(&path, &text).unwrap();

    let cfg = Config::load(&path).unwrap_or_else(|e| panic!("{instance} does not parse: {e}"));
    match cfg.ipc_endpoint() {
        Ok(_) => {}
        Err(e) => explain_refusal(instance, &cfg, &e),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A shipped config that the deployment target (Linux) runs must validate
/// outright there — any refusal is the regression this test exists to catch.
#[cfg(unix)]
fn explain_refusal(instance: &str, _cfg: &Config, e: &ConfigError) -> ! {
    panic!("{instance} must validate on its own platform: {e}");
}

/// Off Unix the one tolerable refusal is "this platform has no Unix domain
/// sockets" for a config that deliberately picks the prod form. A port
/// collision, or a bad address, is still a regression — and note that a dev2
/// that lost its `[ipc]` stanza would fall back to the TCP default and so be
/// caught right here.
#[cfg(not(unix))]
fn explain_refusal(instance: &str, cfg: &Config, e: &ConfigError) {
    assert!(
        matches!(e, ConfigError::BadIpcListen { .. }),
        "{instance}: unexpected refusal: {e}"
    );
    assert!(
        cfg.ipc.listen.parse::<std::net::SocketAddr>().is_err(),
        "{instance}: a TCP [ipc].listen must validate on every platform, not \
         be refused: {e}"
    );
}

#[test]
fn the_shipped_dev1_and_dev2_configs_validate() {
    let _serial = serialized();
    validate_shipped("dev1");
    validate_shipped("dev2");
}
