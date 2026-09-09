//! **The `tls` feature actually serves** (WS24-S4, closes fu10-1).
//!
//! `serve_tls` exists in two halves: the real one behind `#[cfg(feature =
//! "tls")]` and a stub that returns an error. Nothing in CI ever compiled the
//! real half, so a rename in `axum-server`, a rustls provider change or a
//! mistake in the PEM loading path was discovered at redeploy on the VPS —
//! the one place where `web.tls` is set — rather than on the push that caused
//! it. This file is the compile-and-run proof: it mints a certificate, starts
//! the native-TLS listener and completes a real handshake against it.
//!
//! The certificate is generated per run rather than committed. A committed PEM
//! turns CI red at its own `notAfter` with no code change in sight, and the
//! failure looks like a TLS bug rather than an expiry.
//!
//! Port discovery needs care. `serve_with_shutdown_bound` signals its `bound`
//! channel with the *configured* `listen` on the TLS branch, not the bound
//! address, because the socket is bound inside `axum-server` (documented at
//! `src/app.rs`). So `127.0.0.1:0` would hand this test port 0 and there would
//! be nothing to connect to: the test pre-binds an ephemeral port itself,
//! reads it back, drops the listener and passes the concrete address in.

#![cfg(feature = "tls")]

mod common;

use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::{MockWebHooks, test_config};
use mush_net_web::{TlsConfig, WebConfig};

/// A throwaway directory for the generated PEM pair, removed on drop.
///
/// `tempfile` is not a workspace dependency and this needs two files in one
/// directory for the length of one test, so it is cheaper to spell out than to
/// add a crate for.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mush-net-web-{tag}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("a temp directory for the generated certificate");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Mint a self-signed pair valid for `localhost` and `127.0.0.1`, write it as
/// two PEM files and hand back the [`TlsConfig`] that names them.
fn self_signed(dir: &TempDir) -> TlsConfig {
    let issued =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("rcgen mints a self-signed pair");
    let cert = dir.0.join("cert.pem");
    let key = dir.0.join("key.pem");
    fs::write(&cert, issued.cert.pem()).expect("write the certificate");
    fs::write(&key, issued.signing_key.serialize_pem()).expect("write the private key");
    TlsConfig { cert, key }
}

/// An ephemeral port that is free right now. The TLS branch binds the socket
/// itself, so the address has to be concrete before the server starts.
async fn free_local_port() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("the bound address");
    drop(listener);
    addr
}

#[tokio::test]
async fn serve_tls_binds_and_answers_healthz_over_tls() {
    let dir = TempDir::new("tls-serve");
    let tls = self_signed(&dir);
    let listen = free_local_port().await;

    let config = WebConfig {
        listen,
        tls: Some(tls),
        ..test_config()
    };
    let hooks = Arc::new(MockWebHooks::new());
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        mush_net_web::serve_with_shutdown_bound(hooks, config, Some(bound_tx), async move {
            let _ = stop_rx.await;
        })
        .await
    });

    // The TLS branch echoes the configured address, not a bound one — that is
    // the documented behaviour this test is written around.
    let signalled = bound_rx.await.expect("the bound channel is signalled");
    assert_eq!(
        signalled, listen,
        "the TLS branch signals the configured listen address"
    );

    // A no-verify client: the certificate is self-signed by construction, and
    // what is under test is that the handshake and the router work at all.
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("a reqwest client that tolerates a self-signed certificate");
    let url = format!("https://{listen}/healthz");

    // `bound` fires before `axum_server` has the socket; retry the connect.
    let mut last = None;
    let mut response = None;
    for _ in 0..100 {
        match client.get(&url).send().await {
            Ok(resp) => {
                response = Some(resp);
                break;
            }
            Err(err) => {
                last = Some(err);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    let response = response.unwrap_or_else(|| panic!("no TLS handshake completed in 5s: {last:?}"));

    assert_eq!(response.status(), 200, "GET /healthz over TLS");
    let body: serde_json::Value = response.json().await.expect("healthz answers JSON");
    assert_eq!(body["ok"], true, "healthz body: {body}");

    let _ = stop_tx.send(());
    let outcome = tokio::time::timeout(Duration::from_secs(10), server).await;
    let outcome = outcome.expect("the TLS server stops on the shutdown signal");
    outcome
        .expect("the server task did not panic")
        .expect("the TLS server exits cleanly");
}

#[tokio::test]
async fn serve_tls_refuses_a_certificate_path_that_is_not_there() {
    // The other half of fu10-1's drift: the cert-loading path. A missing PEM
    // must surface as an error out of `serve`, not a panic inside the world's
    // sibling task.
    let listen = free_local_port().await;
    let config = WebConfig {
        listen,
        tls: Some(TlsConfig {
            cert: std::env::temp_dir().join("mush-net-web-absent-cert.pem"),
            key: std::env::temp_dir().join("mush-net-web-absent-key.pem"),
        }),
        ..test_config()
    };
    let hooks = Arc::new(MockWebHooks::new());
    let err = mush_net_web::serve_with_shutdown(hooks, config, std::future::pending())
        .await
        .expect_err("a missing certificate cannot serve");
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::NotFound,
        "the PEM loader reports the missing file: {err}"
    );
}
