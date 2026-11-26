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
use mush_storykit::within;

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
    let signalled = bound_rx.await.expect("the bound channel is signaled");
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

    // `bound` fires before `axum_server` has the socket; ask again until a
    // handshake completes. The failure prints the last refusal.
    let response =
        mush_storykit::wait_for(mush_storykit::STEP_TIMEOUT, "a TLS handshake", async || {
            client.get(&url).send().await.map_err(|err| err.to_string())
        })
        .await;

    assert_eq!(
        response.status(),
        503,
        "TLS transport alone does not imply a live world"
    );
    let body: serde_json::Value = response.json().await.expect("healthz answers JSON");
    assert_eq!(body["ok"], false, "healthz body: {body}");

    let _ = stop_tx.send(());
    let outcome = within("the TLS server's stop on the shutdown signal", server).await;
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

#[derive(Debug)]
struct NoVerify;

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Complete a real TLS handshake offering both `h2` and `http/1.1` ALPN, write
/// `payload` as decrypted application bytes, and wait for the server to close
/// the TLS stream. Returns the negotiated ALPN protocol (if any).
async fn tls_send_and_await_close(addr: SocketAddr, payload: &'static [u8]) -> Option<Vec<u8>> {
    within(
        "the TLS probe's close (a wedged permit never closes it)",
        tokio::task::spawn_blocking(move || {
            use std::io::{Read as _, Write as _};

            let _ = rustls::crypto::ring::default_provider().install_default();
            let mut client_cfg = rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth();
            client_cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

            let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
            let conn = rustls::ClientConnection::new(Arc::new(client_cfg), server_name).unwrap();
            let tcp = std::net::TcpStream::connect(addr).unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            tcp.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, tcp);

            // Drive TLS handshake to completion before inspecting ALPN or writing payload.
            while tls.conn.is_handshaking() {
                tls.conn.complete_io(&mut tls.sock).unwrap();
            }
            let alpn = tls.conn.alpn_protocol().map(<[u8]>::to_vec);

            if !payload.is_empty() {
                tls.write_all(payload).unwrap();
                tls.flush().unwrap();
            }

            // Drain until EOF or connection error (server closing after rejection/timeout).
            let mut buf = [0u8; 256];
            loop {
                match tls.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            alpn
        }),
    )
    .await
    .expect("blocking TLS probe task panicked")
}

/// EXT-P1-4 regression: native TLS must enforce `http/1.1` only (never `h2`
/// ALPN or `auto::Builder` `ReadVersion`) so stalled TLS handshakes, 0-byte
/// post-TLS peers, 1-byte `"P"` H2-preface stalls, and full 24-byte
/// `H2_PREFACE` probes all close promptly and release `max_connections` permits.
#[tokio::test]
async fn serve_tls_closes_stalled_handshakes_and_h2_preface_probes_and_releases_permits() {
    use tokio::io::AsyncReadExt as _;

    let dir = TempDir::new("tls-permits");
    let tls = self_signed(&dir);
    let listen = free_local_port().await;

    let config = WebConfig {
        listen,
        tls: Some(tls),
        max_connections: 1,
        header_read_timeout_ms: 200,
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

    let addr = bound_rx.await.expect("TLS listener signalled bound");
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .pool_max_idle_per_host(0)
        .build()
        .expect("reqwest client");
    let url = format!("https://{addr}/healthz");

    let assert_permit_free = || async {
        let resp = within(
            "a subsequent HTTPS request (the permit released)",
            client.get(&url).send(),
        )
        .await
        .expect("subsequent HTTPS request succeeds");
        assert_eq!(resp.status(), 503);
    };

    // 1. Stalled raw TCP connect (0 TLS handshake bytes) times out and frees the single permit.
    let mut stalled = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut buf = [0u8; 16];
    let res = within("the stalled TLS handshake's close", stalled.read(&mut buf)).await;
    assert!(
        matches!(res, Ok(0) | Err(_)),
        "expected EOF/reset on stalled TLS handshake, got {res:?}"
    );
    drop(stalled);
    assert_permit_free().await;

    // 2. Completed TLS handshake + 0 decrypted bytes times out via http1 header_read_timeout.
    let alpn = tls_send_and_await_close(addr, b"").await;
    assert_eq!(
        alpn.as_deref(),
        Some(b"http/1.1".as_slice()),
        "TLS server must negotiate http/1.1 only, never h2"
    );
    assert_permit_free().await;

    // 3. Completed TLS handshake + 1 byte `P` (H2_PREFACE prefix) used to clear
    //    `PermitStream::first_byte_deadline` and wedge `auto::Builder`'s
    //    `ReadVersion` forever; direct http1 times it out promptly.
    let alpn = tls_send_and_await_close(addr, b"P").await;
    assert_eq!(alpn.as_deref(), Some(b"http/1.1".as_slice()));
    assert_permit_free().await;

    // 4. Completed TLS handshake + full 24-byte HTTP/2 preface must be rejected
    //    by HTTP/1 rather than entering HTTP/2 with no timeouts.
    let alpn = tls_send_and_await_close(addr, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n").await;
    assert_eq!(alpn.as_deref(), Some(b"http/1.1".as_slice()));
    assert_permit_free().await;

    let _ = stop_tx.send(());
    within("the TLS server's stop on shutdown", server)
        .await
        .expect("server task did not panic")
        .expect("TLS server exits cleanly");
}
