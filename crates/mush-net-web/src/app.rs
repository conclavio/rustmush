//! Router assembly and the serve entry points.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower_governor::GovernorError;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::config::WebConfig;
use crate::hooks::{WEB_CONN_ID_BASE, WebHooks};
use crate::ip::{client_ip, fold_network_key};
use crate::metrics::Metrics;
use crate::{assets, auth, ws};

/// Hard body limit for `/api/auth/*` endpoints (HIGH-3): login, register, and
/// logout payloads are small credential JSON objects and never need Axum's
/// 2 MiB default.
const AUTH_BODY_LIMIT_BYTES: usize = 4_096;

/// Shared state behind every handler.
#[derive(Clone)]
pub(crate) struct AppState {
    pub hooks: Arc<dyn WebHooks>,
    pub config: Arc<WebConfig>,
    /// Server start, for `/healthz` uptime.
    pub started: Instant,
    /// Next WS conn id (starts at [`WEB_CONN_ID_BASE`], with the hooks'
    /// generation bit set, doc 33 §4.3).
    pub next_conn: Arc<AtomicU64>,
    /// The shared metrics registry (from [`WebHooks::metrics`]) read by
    /// `/healthz` and `/metrics`.
    pub metrics: Arc<Metrics>,
}

/// Builds the complete axum app. Public for tests and embedders; `mushd`
/// normally calls [`serve`] instead.
///
/// The router must be served with connect info
/// (`app.into_make_service_with_connect_info::<SocketAddr>()`) — the
/// rate-limit key and IpGuard checks need the peer address.
pub fn router(hooks: Arc<dyn WebHooks>, config: WebConfig) -> Router {
    build(hooks, config).0
}

/// Periodic maintenance handle for the rate limiter (drops stale per-IP
/// buckets); run by [`serve`].
type LimiterGc = Box<dyn Fn() + Send + Sync>;

pub(crate) fn build(hooks: Arc<dyn WebHooks>, config: WebConfig) -> (Router, Option<LimiterGc>) {
    let metrics = hooks.metrics();
    let first_conn = WEB_CONN_ID_BASE | hooks.conn_id_bit();
    let metrics_enabled = config.metrics_enabled;
    let state = AppState {
        hooks,
        config: Arc::new(config),
        started: Instant::now(),
        next_conn: Arc::new(AtomicU64::new(first_conn)),
        metrics,
    };

    let auth_routes = Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/logout", post(auth::logout))
        .layer(DefaultBodyLimit::max(AUTH_BODY_LIMIT_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::reject_blocked_ip,
        ));
    let (auth_routes, limiter_gc) = rate_limit(auth_routes, &state.config);

    let mut http_ops = Router::new().route("/healthz", get(healthz));
    if metrics_enabled {
        // Gated behind `web.metrics_enabled` (opt-in, default off). Public-safe:
        // non-sensitive aggregates only (see `metrics.rs`).
        http_ops = http_ops.route("/metrics", get(metrics_endpoint));
    }
    let mut non_upgrade = auth_routes.merge(http_ops).fallback(assets::static_handler);
    let req_timeout = Duration::from_millis(state.config.request_timeout_ms);
    if !req_timeout.is_zero() {
        non_upgrade =
            non_upgrade.layer(middleware::from_fn_with_state(req_timeout, request_timeout));
    }
    // Keep `/ws` outside `request_timeout` so long-lived WebSocket upgrades
    // are governed by WS ping/pong and send timeouts instead.
    let ws_routes = Router::new().route("/ws", get(ws::ws_route));

    let app = security_headers(non_upgrade.merge(ws_routes))
        .layer(TraceLayer::new_for_http().make_span_with(scrubbed_span))
        .with_state(state);
    (app, limiter_gc)
}

async fn request_timeout(
    State(timeout): State<Duration>,
    req: Request<Body>,
    next: Next,
) -> Response {
    match tokio::time::timeout(timeout, next.run(req)).await {
        Ok(resp) => resp,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}

/// Baseline security headers on every response (LOW finding: the SPA keeps
/// its session token in `localStorage`, so a CSP backstop against injected
/// script matters). `overriding` — nothing downstream sets these today, but
/// a future handler must not be able to silently weaken them.
///
/// `style-src` needs `'unsafe-inline'`: `RichLine`/`ChannelTabs`/`ChannelView`
/// render server-sent explicit colors ([`mush_proto::Style`], channel hex)
/// through dynamic `style="…"` attributes, not CSS classes — verified
/// against `web/dist` at design time (no inline `<script>`, no `eval`, no
/// external asset hosts, so everything else stays same-origin-only).
/// `connect-src` allows `ws:`/`wss:` explicitly for the game socket rather
/// than relying on `'self'`'s scheme-upgrade matching.
///
/// `script-src` adds `'wasm-unsafe-eval'` (doc 21 cut G.3b): a browser
/// compiles a WebAssembly module only under it, and the cockpit picture is
/// one (the bundle's own, same-origin). It permits compiling WebAssembly and
/// nothing else — not `eval`, not `new Function`, not an inline script.
fn security_headers(router: Router<AppState>) -> Router<AppState> {
    const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
        style-src 'self' 'unsafe-inline'; img-src 'self'; \
        connect-src 'self' ws: wss:; base-uri 'none'; \
        frame-ancestors 'none'; object-src 'none'";
    router
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
}

/// The tight per-network bucket on `/api/auth/*` (design 02 §8.3, HIGH-3):
/// burst `auth_burst`, one element replenished every `auth_replenish_secs`.
/// Always uses [`NetworkKeyExtractor`] so both direct and `trusted_proxies`
/// modes canonicalize IPv4-mapped IPv6 addresses and fold native IPv6 to `/64`
/// (matching `IpGuard`). Either knob at 0 disables the layer (tests only).
fn rate_limit(
    router: Router<AppState>,
    config: &WebConfig,
) -> (Router<AppState>, Option<LimiterGc>) {
    if config.auth_burst == 0 || config.auth_replenish_secs == 0 {
        return (router, None);
    }
    let key_extractor = NetworkKeyExtractor {
        trusted_proxies: Arc::from(config.trusted_proxies.as_slice()),
    };
    let Some(governor_cfg) = GovernorConfigBuilder::default()
        .key_extractor(key_extractor)
        .per_second(config.auth_replenish_secs)
        .burst_size(config.auth_burst)
        .finish()
    else {
        return (router, None);
    };
    let governor = Arc::new(governor_cfg);
    let gc: LimiterGc = {
        let governor = Arc::clone(&governor);
        Box::new(move || governor.limiter().retain_recent())
    };
    (router.layer(GovernorLayer::new(governor)), Some(gc))
}

/// [`KeyExtractor`] mirroring [`client_ip`] and `IpGuard`'s `/64` IPv6 / `/32`
/// IPv4 network folding (MEDIUM-LOW finding, design 02 §6.5, HIGH-3).
///
/// Handles both direct connections (`trusted_proxies` empty) and reverse-proxy
/// hops (`trusted_proxies` non-empty): untrusted direct peers can never spoof
/// `X-Forwarded-For`, IPv4-mapped IPv6 addresses fold onto their IPv4 address,
/// and native IPv6 addresses fold to their `/64` prefix so rotating host IDs
/// within one allocation shares one auth rate-limit bucket.
#[derive(Clone)]
struct NetworkKeyExtractor {
    trusted_proxies: Arc<[IpAddr]>,
}

impl KeyExtractor for NetworkKeyExtractor {
    type Key = IpAddr;

    // `tower_governor`'s `tracing` feature is pinned on unconditionally by
    // the workspace `Cargo.toml`, which makes `name`/`key_name` required
    // trait items here — not optional the way they are inside
    // `tower_governor` itself (whose own `#[cfg(feature = "tracing")]` reads
    // *its* Cargo features, not ours). No `#[cfg]` on either method: this
    // impl only needs to compile against the one fixed dependency config
    // this workspace pins.
    fn name(&self) -> &'static str {
        "client network"
    }

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or(GovernorError::UnableToExtractKey)?
            .0
            .ip();
        let resolved = client_ip(peer, req.headers(), &self.trusted_proxies);
        Ok(fold_network_key(resolved))
    }

    fn key_name(&self, key: &Self::Key) -> Option<String> {
        Some(key.to_string())
    }
}

/// Trace span for every request: **method + path only** — never the query
/// string (`/ws?token=…` must not reach the logs; design 02 §6.2).
fn scrubbed_span(request: &Request<Body>) -> tracing::Span {
    tracing::info_span!(
        "http",
        method = %request.method(),
        path = %request.uri().path(),
    )
}

/// `GET /healthz` → enriched liveness JSON: `ok`, `uptime_s`, `version`,
/// `connections`, `loop_lag_ms`, `world_loop_unixtime`, `store_ok`.
/// Non-sensitive aggregates only. Read lock-free straight from the atomics —
/// this endpoint is unauthenticated and unthrottled, so it must not take the
/// LLM mutex or allocate a full snapshot per hit.
async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = state.started.elapsed().as_secs();
    let health = state.metrics.health_json(uptime, env!("CARGO_PKG_VERSION"));
    let status = if health["ok"] == true {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(health))
}

/// `GET /metrics` → Prometheus text exposition. Registered only when
/// `web.metrics_enabled` is set (opt-in, default off). Exposes only aggregate
/// counters/gauges/histograms — never per-player data, keys, or tokens.
async fn metrics_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = state.started.elapsed().as_secs();
    let body = state.metrics.snapshot().to_prometheus(uptime);
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
        )],
        body,
    )
}

/// Serves until the process ends. See
/// [`serve_with_shutdown`] for graceful stops.
pub async fn serve(hooks: Arc<dyn WebHooks>, config: WebConfig) -> std::io::Result<()> {
    serve_with_shutdown(hooks, config, std::future::pending()).await
}

/// Binds `config.listen` and serves until `shutdown` resolves (graceful:
/// in-flight requests finish; WS tasks notice their sockets closing).
///
/// With `config.tls` set, terminates TLS natively via rustls (requires the
/// `tls` cargo feature; ring provider — no C toolchain) and serves HTTP/1.1
/// directly with the same connection cap and header timeout as plain HTTP.
/// Without it, plain HTTP — the recommended prod setup keeps a reverse proxy
/// (Caddy/nginx) in front and `trusted_proxies` pointed at it.
pub async fn serve_with_shutdown(
    hooks: Arc<dyn WebHooks>,
    config: WebConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    serve_with_shutdown_bound(hooks, config, None, shutdown).await
}

/// Like [`serve_with_shutdown`], but signals `bound` with the actually-bound
/// listen address once the listener is up (mirrors the SSH listener's
/// ready-channel pattern). A caller that binds `127.0.0.1:0` reads the
/// OS-assigned port back through it; the split gateway uses this to hand its
/// integration tests the real web address.
pub async fn serve_with_shutdown_bound(
    hooks: Arc<dyn WebHooks>,
    config: WebConfig,
    bound: Option<tokio::sync::oneshot::Sender<SocketAddr>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listener = std::net::TcpListener::bind(config.listen)?;
    serve_with_shutdown_bound_on(hooks, config, listener, bound, shutdown).await
}

/// [`serve_with_shutdown_bound`] on a listener the caller bound (doc 33
/// §4.4): the split gateway binds its web port through
/// `mush_drain::listen::bind_std`, with `SO_REUSEPORT`, so two generations
/// share the port while one drains, and the recipe lives in one place.
/// `config.listen` is not consulted; the listener is put in non-blocking
/// mode here, and both plain HTTP and `[web.tls]` serve directly on the same
/// socket.
pub async fn serve_with_shutdown_bound_on(
    hooks: Arc<dyn WebHooks>,
    config: WebConfig,
    listener: std::net::TcpListener,
    bound: Option<tokio::sync::oneshot::Sender<SocketAddr>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let actual = listener.local_addr()?;
    let tls = config.tls.clone();
    let max_connections = config.max_connections.max(1);
    let header_timeout = Duration::from_millis(config.header_read_timeout_ms);
    let conn_sem = Arc::new(Semaphore::new(max_connections));
    let (app, limiter_gc) = build(hooks, config);

    // Rate-limiter GC: drop per-IP buckets that have fully replenished.
    let (stop_gc, mut gc_stopped) = tokio::sync::watch::channel(false);
    if let Some(gc) = limiter_gc {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = tick.tick() => gc(),
                    _ = gc_stopped.changed() => return,
                }
            }
        });
    }

    let result = match tls {
        None => {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            tracing::info!(listen = %actual, "web server listening (http)");
            if let Some(tx) = bound {
                let _ = tx.send(actual);
            }
            serve_http(listener, app, conn_sem, header_timeout, shutdown).await
        }
        Some(tls_config) => {
            serve_tls(
                listener,
                actual,
                tls_config,
                app,
                conn_sem,
                header_timeout,
                bound,
                shutdown,
            )
            .await
        }
    };
    let _ = stop_gc.send(true);
    result
}

/// Wraps a stream so the global TCP connection semaphore permit lives for the
/// entire lifetime of the connection (including after WebSocket upgrades).
struct PermitStream<S> {
    inner: S,
    _permit: OwnedSemaphorePermit,
}

impl<S> PermitStream<S> {
    fn new(inner: S, permit: OwnedSemaphorePermit) -> Self {
        Self {
            inner,
            _permit: permit,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PermitStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PermitStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, bufs)
    }
}

fn http1_builder(header_timeout: Duration) -> hyper::server::conn::http1::Builder {
    let mut http1 = hyper::server::conn::http1::Builder::new();
    http1.timer(TokioTimer::new());
    http1.keep_alive(true);
    if !header_timeout.is_zero() {
        http1.header_read_timeout(header_timeout);
    }
    http1
}

async fn serve_http1_conn<S>(
    stream: PermitStream<S>,
    remote_addr: SocketAddr,
    app: Router,
    http1: hyper::server::conn::http1::Builder,
    mut conn_shutdown: tokio::sync::watch::Receiver<bool>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let io = TokioIo::new(stream);
    let service = TowerToHyperService::new(app.layer(Extension(ConnectInfo(remote_addr))));
    let mut conn = std::pin::pin!(http1.serve_connection(io, service).with_upgrades());
    tokio::select! {
        res = conn.as_mut() => {
            if let Err(err) = res {
                tracing::debug!(peer = %remote_addr, error = %err, "http connection closed");
            }
        }
        _ = conn_shutdown.changed() => {
            conn.as_mut().graceful_shutdown();
            let _ = tokio::time::timeout(Duration::from_secs(10), conn).await;
        }
    }
}

async fn serve_http(
    listener: tokio::net::TcpListener,
    app: Router,
    conn_sem: Arc<Semaphore>,
    header_timeout: Duration,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    let http1 = http1_builder(header_timeout);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut shutdown = std::pin::pin!(shutdown);
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        while tasks.try_join_next().is_some() {}

        let accepted = tokio::select! {
            biased;
            () = &mut shutdown => break,
            res = listener.accept() => res,
        };
        let (stream, remote_addr) = match accepted {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!(error = %err, "http accept error");
                tokio::time::sleep(Duration::from_millis(25)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&conn_sem).try_acquire_owned() else {
            tracing::warn!(peer = %remote_addr, "web connection cap reached; dropping socket");
            drop(stream);
            continue;
        };
        let _ = stream.set_nodelay(true);
        let http1 = http1.clone();
        let app = app.clone();
        let conn_shutdown = shutdown_rx.clone();
        tasks.spawn(async move {
            serve_http1_conn(
                PermitStream::new(stream, permit),
                remote_addr,
                app,
                http1,
                conn_shutdown,
            )
            .await;
        });
    }

    drop(listener);
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    Ok(())
}

#[cfg(feature = "tls")]
#[allow(clippy::too_many_arguments)]
async fn serve_tls(
    listener: std::net::TcpListener,
    listen: SocketAddr,
    tls: crate::config::TlsConfig,
    app: Router,
    conn_sem: Arc<Semaphore>,
    header_timeout: Duration,
    bound: Option<tokio::sync::oneshot::Sender<SocketAddr>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    use axum_server::accept::Accept as _;

    // rustls needs a process-level crypto provider. Ring keeps the
    // workspace free of C-toolchain babysitting (aws-lc-sys wants
    // CMake/NASM); ignore the error if something already installed one.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let loaded = axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key).await?;
    // Enforce HTTP/1.1-only ALPN and feed decrypted streams directly into
    // `hyper::server::conn::http1::Builder` (never `auto::Builder` / HTTP/2):
    // `header_read_timeout` arms immediately on the first read after TLS
    // handshake, and any HTTP/2 preface bytes are rejected/timed out instead
    // of holding connection permits indefinitely (EXT-P1-4).
    let mut server_config = (*loaded.get_inner()).clone();
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let rustls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config));
    let handshake_timeout = if header_timeout.is_zero() {
        Duration::from_secs(24 * 60 * 60)
    } else {
        header_timeout
    };
    let acceptor = axum_server::tls_rustls::RustlsAcceptor::new(rustls_config)
        .handshake_timeout(handshake_timeout);

    let listener = tokio::net::TcpListener::from_std(listener)?;
    tracing::info!(%listen, "web server listening (https)");
    if let Some(tx) = bound {
        let _ = tx.send(listen);
    }

    let http1 = http1_builder(header_timeout);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut shutdown = std::pin::pin!(shutdown);
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        while tasks.try_join_next().is_some() {}

        let accepted = tokio::select! {
            biased;
            () = &mut shutdown => break,
            res = listener.accept() => res,
        };
        let (stream, remote_addr) = match accepted {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!(error = %err, "https accept error");
                tokio::time::sleep(Duration::from_millis(25)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&conn_sem).try_acquire_owned() else {
            tracing::warn!(peer = %remote_addr, "web connection cap reached; dropping socket");
            drop(stream);
            continue;
        };
        let _ = stream.set_nodelay(true);
        let acceptor = acceptor.clone();
        let http1 = http1.clone();
        let app = app.clone();
        let mut conn_shutdown = shutdown_rx.clone();
        tasks.spawn(async move {
            let tls_stream = tokio::select! {
                biased;
                _ = conn_shutdown.changed() => return,
                res = acceptor.accept(stream, ()) => match res {
                    Ok((tls_stream, ())) => tls_stream,
                    Err(err) => {
                        tracing::debug!(peer = %remote_addr, error = %err, "tls handshake failed");
                        return;
                    }
                },
            };
            serve_http1_conn(
                PermitStream::new(tls_stream, permit),
                remote_addr,
                app,
                http1,
                conn_shutdown,
            )
            .await;
        });
    }

    drop(listener);
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    Ok(())
}

#[cfg(not(feature = "tls"))]
#[allow(clippy::too_many_arguments)]
async fn serve_tls(
    _listener: std::net::TcpListener,
    _listen: SocketAddr,
    _tls: crate::config::TlsConfig,
    _app: Router,
    _conn_sem: Arc<Semaphore>,
    _header_timeout: Duration,
    _bound: Option<tokio::sync::oneshot::Sender<SocketAddr>>,
    _shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "web.tls is configured but mush-net-web was built without the `tls` feature",
    ))
}
