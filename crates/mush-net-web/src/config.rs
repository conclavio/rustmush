//! [`WebConfig`] — everything the web ingress needs from `mushd.toml`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Native-TLS material (the optional `tls` cargo feature; prod normally
/// terminates TLS at a reverse proxy instead — see the crate docs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsConfig {
    /// PEM certificate chain file.
    pub cert: PathBuf,
    /// PEM private key file.
    pub key: PathBuf,
}

/// Configuration for [`serve`](crate::serve) / [`router`](crate::router).
///
/// Every field has a serde default, so a `[web]` TOML table only needs the
/// values it wants to override.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Listen address. Default `127.0.0.1:8080` (the vite dev proxy's
    /// target); bind a public address or put a reverse proxy in front for
    /// prod.
    pub listen: SocketAddr,
    /// Server software string sent in the `hello` frame.
    pub server_name: String,
    /// Reverse-proxy addresses whose `X-Forwarded-For` is trusted. When
    /// empty (default), forwarded headers are ignored entirely and the peer
    /// address is the client address.
    pub trusted_proxies: Vec<IpAddr>,
    /// Feature identifiers advertised in the `hello` frame (e.g. `"mail"`,
    /// `"channels"`); `mushd` fills these from its module registry.
    pub features: Vec<String>,
    /// Panel feed ids advertised in the `hello` frame.
    pub panels: Vec<String>,
    /// Native TLS material. `Some` requires the `tls` cargo feature;
    /// without it [`serve`](crate::serve) refuses to start.
    pub tls: Option<TlsConfig>,
    /// Auth-route rate limit: bucket capacity (requests that may burst).
    /// Setting this or [`auth_replenish_secs`](Self::auth_replenish_secs)
    /// to `0` disables the limiter (tests only — never in prod).
    pub auth_burst: u32,
    /// Auth-route rate limit: seconds to replenish one bucket element.
    pub auth_replenish_secs: u64,
    /// Bounded per-connection outbound envelope queue (backpressure; the
    /// session layer's `try_send` drops-and-flags when it is full).
    pub send_queue: usize,
    /// App-level keepalive ping interval, milliseconds.
    pub ping_interval_ms: u64,
    /// Unanswered pings tolerated before the connection is closed as dead.
    pub max_missed_pongs: u32,
    /// How long one WS send may block before the client is closed as too
    /// slow, milliseconds.
    pub send_timeout_ms: u64,
    /// Maximum inbound WS message size, bytes; oversize is a protocol
    /// error close.
    pub max_frame_bytes: usize,
    /// Whether `GET /metrics` (Prometheus text exposition) is served.
    /// Default **`false`**: the endpoint binds to this same (possibly public)
    /// listener with no auth, so it is opt-in — an operator running Prometheus
    /// sets it `true` and firewalls the path / keeps it behind the reverse
    /// proxy. It exposes only non-sensitive aggregates (never per-player data
    /// or keys). `/healthz` is always served.
    pub metrics_enabled: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080),
            server_name: format!("rustmush {}", env!("CARGO_PKG_VERSION")),
            trusted_proxies: Vec::new(),
            features: Vec::new(),
            panels: Vec::new(),
            tls: None,
            auth_burst: 5,
            auth_replenish_secs: 30,
            send_queue: 256,
            ping_interval_ms: 25_000,
            max_missed_pongs: 2,
            send_timeout_ms: 10_000,
            max_frame_bytes: mush_proto::MAX_INVOKE_BYTES,
            metrics_enabled: false,
        }
    }
}
