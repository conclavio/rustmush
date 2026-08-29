//! Router assembly and the serve entry points.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router, extract::State};
use tower_governor::GovernorError;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::config::WebConfig;
use crate::hooks::{WEB_CONN_ID_BASE, WebHooks};
use crate::ip::client_ip;
use crate::metrics::Metrics;
use crate::{assets, auth, ws};

/// Shared state behind every handler.
#[derive(Clone)]
pub(crate) struct AppState {
    pub hooks: Arc<dyn WebHooks>,
    pub config: Arc<WebConfig>,
    /// Server start, for `/healthz` uptime.
    pub started: Instant,
    /// Next WS conn id (starts at [`WEB_CONN_ID_BASE`]).
    pub next_conn: Arc<AtomicU64>,
    /// The shared metrics registry (from [`WebHooks::metrics`]) read by
    /// `/healthz` and `/metrics`.
    pub metrics: Arc<Metrics>,
}

/// Builds the complete axum app. Public for tests and embedders; `mushd`
/// normally calls [`serve`](crate::serve) instead.
///
/// The router must be served with connect info
/// (`app.into_make_service_with_connect_info::<SocketAddr>()`) — the
/// rate-limit key and IpGuard checks need the peer address.
pub fn router(hooks: Arc<dyn WebHooks>, config: WebConfig) -> Router {
    build(hooks, config).0
}

/// Periodic maintenance handle for the rate limiter (drops stale per-IP
/// buckets); run by [`serve`](crate::serve).
type LimiterGc = Box<dyn Fn() + Send + Sync>;

pub(crate) fn build(hooks: Arc<dyn WebHooks>, config: WebConfig) -> (Router, Option<LimiterGc>) {
    let metrics = hooks.metrics();
    let metrics_enabled = config.metrics_enabled;
    let state = AppState {
        hooks,
        config: Arc::new(config),
        started: Instant::now(),
        next_conn: Arc::new(AtomicU64::new(WEB_CONN_ID_BASE)),
        metrics,
    };

    let auth_routes = Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/logout", post(auth::logout));
    let (auth_routes, limiter_gc) = rate_limit(auth_routes, &state.config);

    let mut ops = Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws::ws_route));
    if metrics_enabled {
        // Gated behind `web.metrics_enabled` (opt-in, default off). Public-safe:
        // non-sensitive aggregates only (see `metrics.rs`).
        ops = ops.route("/metrics", get(metrics_endpoint));
    }

    let app = security_headers(auth_routes.merge(ops).fallback(assets::static_handler))
        .layer(TraceLayer::new_for_http().make_span_with(scrubbed_span))
        .with_state(state);
    (app, limiter_gc)
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
fn security_headers(router: Router<AppState>) -> Router<AppState> {
    const CSP: &str = "default-src 'self'; script-src 'self'; \
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

/// The tight per-IP bucket on `/api/auth/*` (design 02 §8.3): burst
/// `auth_burst`, one element replenished every `auth_replenish_secs`.
/// [`TrustedPeerKeyExtractor`] (forwarded headers, gated on a trusted direct
/// peer) is installed **only** when `trusted_proxies` is configured;
/// otherwise the peer IP is the key. Either knob at 0 disables the layer
/// (tests only).
fn rate_limit(
    router: Router<AppState>,
    config: &WebConfig,
) -> (Router<AppState>, Option<LimiterGc>) {
    if config.auth_burst == 0 || config.auth_replenish_secs == 0 {
        return (router, None);
    }
    if config.trusted_proxies.is_empty() {
        let governor = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(config.auth_replenish_secs)
                .burst_size(config.auth_burst)
                .finish()
                .expect("nonzero rate-limit parameters"),
        );
        let gc: LimiterGc = {
            let governor = Arc::clone(&governor);
            Box::new(move || governor.limiter().retain_recent())
        };
        (router.layer(GovernorLayer::new(governor)), Some(gc))
    } else {
        let key_extractor = TrustedPeerKeyExtractor {
            trusted_proxies: Arc::from(config.trusted_proxies.as_slice()),
        };
        let governor = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(key_extractor)
                .per_second(config.auth_replenish_secs)
                .burst_size(config.auth_burst)
                .finish()
                .expect("nonzero rate-limit parameters"),
        );
        let gc: LimiterGc = {
            let governor = Arc::clone(&governor);
            Box::new(move || governor.limiter().retain_recent())
        };
        (router.layer(GovernorLayer::new(governor)), Some(gc))
    }
}

/// [`KeyExtractor`] mirroring [`client_ip`]'s trust check (MEDIUM-LOW
/// finding, design 02 §6.5): `tower_governor`'s built-in
/// `SmartIpKeyExtractor` honors `X-Forwarded-For` from **any** direct peer
/// once `trusted_proxies` is non-empty, so an attacker connecting straight
/// to this server (not through the configured proxy) could set the header
/// themselves and spread requests across as many rate-limit buckets as they
/// like, defeating the `/api/auth/*` bucket entirely. This extractor keys
/// on [`client_ip`] instead — the same trusted-direct-peer gate `IpGuard`
/// uses — so a forged header from an untrusted peer is ignored here exactly
/// as it is for ban/tempban purposes.
#[derive(Clone)]
struct TrustedPeerKeyExtractor {
    trusted_proxies: Arc<[IpAddr]>,
}

impl KeyExtractor for TrustedPeerKeyExtractor {
    type Key = IpAddr;

    // `tower_governor`'s `tracing` feature is pinned on unconditionally by
    // the workspace `Cargo.toml`, which makes `name`/`key_name` required
    // trait items here — not optional the way they are inside
    // `tower_governor` itself (whose own `#[cfg(feature = "tracing")]` reads
    // *its* Cargo features, not ours). No `#[cfg]` on either method: this
    // impl only needs to compile against the one fixed dependency config
    // this workspace pins.
    fn name(&self) -> &'static str {
        "trusted-peer IP"
    }

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or(GovernorError::UnableToExtractKey)?
            .0
            .ip();
        Ok(client_ip(peer, req.headers(), &self.trusted_proxies))
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
    Json(state.metrics.health_json(uptime, env!("CARGO_PKG_VERSION")))
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
/// [`serve_with_shutdown`](crate::serve_with_shutdown) for graceful stops.
pub async fn serve(hooks: Arc<dyn WebHooks>, config: WebConfig) -> std::io::Result<()> {
    serve_with_shutdown(hooks, config, std::future::pending()).await
}

/// Binds `config.listen` and serves until `shutdown` resolves (graceful:
/// in-flight requests finish; WS tasks notice their sockets closing).
///
/// With `config.tls` set, terminates TLS natively via `axum-server` +
/// rustls (requires the `tls` cargo feature; ring provider — no C
/// toolchain). Without it, plain HTTP — the recommended prod setup keeps a
/// reverse proxy (Caddy/nginx) in front and `trusted_proxies` pointed at
/// it.
pub async fn serve_with_shutdown(
    hooks: Arc<dyn WebHooks>,
    config: WebConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listen = config.listen;
    let tls = config.tls.clone();
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
            let listener = tokio::net::TcpListener::bind(listen).await?;
            tracing::info!(%listen, "web server listening (http)");
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown)
            .await
        }
        Some(tls_config) => serve_tls(listen, tls_config, app, shutdown).await,
    };
    let _ = stop_gc.send(true);
    result
}

#[cfg(feature = "tls")]
async fn serve_tls(
    listen: SocketAddr,
    tls: crate::config::TlsConfig,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    // rustls needs a process-level crypto provider. Ring keeps the
    // workspace free of C-toolchain babysitting (aws-lc-sys wants
    // CMake/NASM); ignore the error if something already installed one.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rustls_config =
        axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key).await?;
    let handle = axum_server::Handle::new();
    {
        let handle = handle.clone();
        tokio::spawn(async move {
            shutdown.await;
            handle.graceful_shutdown(Some(Duration::from_secs(10)));
        });
    }
    tracing::info!(%listen, "web server listening (https)");
    axum_server::bind_rustls(listen, rustls_config)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

#[cfg(not(feature = "tls"))]
async fn serve_tls(
    _listen: SocketAddr,
    _tls: crate::config::TlsConfig,
    _app: Router,
    _shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "web.tls is configured but mush-net-web was built without the `tls` feature",
    ))
}
