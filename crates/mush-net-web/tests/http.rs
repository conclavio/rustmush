//! Route tests over real HTTP: auth flow, rate limiting, IpGuard, healthz,
//! and the embedded-SPA fallback.

mod common;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use common::{ACCOUNT, MockWebHooks, PASSWORD, TOKEN, spawn_server, test_config};
use mush_net_web::{LoginErr, LoginOk, LogoutErr, RegisterErr, WebConfig};
use mush_storykit::{STEP_TIMEOUT, wait_until, within};
use serde_json::{Value, json};

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Whether a fresh connection to `addr` is served: `/healthz` answers 200.
/// Over a connection cap the server drops the socket unanswered, so `false`
/// is "no slot free yet", and the caller asks again.
async fn served(addr: std::net::SocketAddr) -> bool {
    client()
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .is_ok_and(|resp| resp.status() == 200)
}

/// **A slot is free** (doc 51 §3.5): wait for the fact, a fresh connection
/// served, not for a span after a socket was dropped. A dropped socket's slot
/// comes back when the server's connection task sees the end of it, which no
/// client can time.
async fn until_served(addr: std::net::SocketAddr, what: &str) {
    wait_until(STEP_TIMEOUT, what, async || served(addr).await).await;
}

/// Reads frames until the server's first keepalive ping and hands it back.
/// The ping is the server's own mark of time: the pump sends it one
/// `ping_interval_ms` after the socket was attached, so a ping read proves the
/// socket was open that long, with no span chosen by the story.
async fn first_ping<S>(ws: &mut S) -> mush_proto::Envelope
where
    S: futures::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    use tokio_tungstenite::tungstenite::Message;
    loop {
        let frame = within("the first keepalive ping", futures::StreamExt::next(ws))
            .await
            .expect("the socket ended before its first ping")
            .expect("ws error before the first ping");
        match frame {
            Message::Text(text) => {
                let env: mush_proto::Envelope = serde_json::from_str(text.as_str()).unwrap();
                if matches!(env.event, mush_proto::OutputEvent::Ping { .. }) {
                    return env;
                }
            }
            Message::Close(close) => panic!("closed before the first ping: {close:?}"),
            _ => {}
        }
    }
}

async fn post_json(url: &str, body: Value) -> reqwest::Response {
    client().post(url).json(&body).send().await.unwrap()
}

#[tokio::test]
async fn login_ok_returns_token_and_characters() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let resp = post_json(
        &format!("http://{addr}/api/auth/login"),
        json!({"account": ACCOUNT, "password": PASSWORD}),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["token"], TOKEN);
    assert_eq!(body["account"], ACCOUNT);
    assert_eq!(body["characters"][0]["key"], 7);
    assert_eq!(body["characters"][0]["name"], "Nick");
    assert!(hooks.state().auth_failed.is_empty());
}

#[tokio::test]
async fn login_bad_creds_is_uniform_401_and_counts_auth_failure() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/login");

    // Wrong password vs. unknown account: identical status and body.
    let wrong_pw = post_json(&url, json!({"account": ACCOUNT, "password": "nope"})).await;
    assert_eq!(wrong_pw.status(), 401);
    let body_a = wrong_pw.text().await.unwrap();

    let unknown = post_json(&url, json!({"account": "ghost", "password": "x"})).await;
    assert_eq!(unknown.status(), 401);
    let body_b = unknown.text().await.unwrap();

    assert_eq!(body_a, body_b, "401 bodies must be indistinguishable");
    assert_eq!(
        hooks.state().auth_failed,
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST); 2]
    );
}

#[tokio::test]
async fn login_banned_and_ratelimited_map_to_403_and_429() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/login");
    let body = json!({"account": ACCOUNT, "password": PASSWORD});

    hooks.state().login_err = Some(LoginErr::Banned);
    assert_eq!(post_json(&url, body.clone()).await.status(), 403);

    hooks.state().login_err = Some(LoginErr::RateLimited);
    assert_eq!(post_json(&url, body).await.status(), 429);
    // Neither maps to auth_failed (only BadCreds counts).
    assert!(hooks.state().auth_failed.is_empty());
}

#[tokio::test]
async fn auth_routes_rate_limit_429_after_burst() {
    let hooks = Arc::new(MockWebHooks::new());
    // Default knobs: burst 5, one element per 30 s, keyed by peer IP.
    let addr = spawn_server(hooks, test_config()).await;
    let url = format!("http://{addr}/api/auth/login");
    let body = json!({"account": "ghost", "password": "x"});

    for i in 0..5 {
        let resp = post_json(&url, body.clone()).await;
        assert_eq!(resp.status(), 401, "request {i} should pass the limiter");
    }
    let resp = post_json(&url, body).await;
    assert_eq!(resp.status(), 429, "burst exhausted -> governor 429");
}

#[tokio::test]
async fn register_maps_results() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/register");
    let req = json!({"account": "newbie", "password": "hunter22", "char_name": "Newbie"});

    hooks.state().register_result = Some(Ok(LoginOk {
        token: "tok-newbie".into(),
        account: "newbie".into(),
        characters: vec![],
    }));
    let resp = post_json(&url, req.clone()).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["token"], "tok-newbie");

    hooks.state().register_result = Some(Err(RegisterErr::Taken));
    assert_eq!(post_json(&url, req.clone()).await.status(), 409);

    hooks.state().register_result = Some(Err(RegisterErr::BadName("too short".into())));
    let resp = post_json(&url, req.clone()).await;
    assert_eq!(resp.status(), 422);
    assert_eq!(resp.json::<Value>().await.unwrap()["error"], "too short");

    hooks.state().register_result = Some(Err(RegisterErr::Weak));
    assert_eq!(post_json(&url, req).await.status(), 422);
}

#[tokio::test]
async fn logout_revokes_bearer_token() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/logout");

    let resp = client()
        .post(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    assert_eq!(hooks.state().logouts, vec![TOKEN.to_string()]);

    // Missing header is a 400, not a panic or a 204.
    let resp = client().post(&url).send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn logout_returns_503_when_durable_token_delete_fails() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().logout_err = Some(LogoutErr::Unavailable);
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/logout");

    let resp = client()
        .post(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        503,
        "failed durable token delete must never return 204"
    );
    let body: Value = resp.json().await.unwrap();
    assert!(body["error"].is_string(), "error body: {body}");
    assert_eq!(hooks.state().logouts, vec![TOKEN.to_string()]);
}

#[tokio::test]
async fn blocked_ip_gets_403_before_auth() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks
        .state()
        .blocked
        .insert(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let resp = post_json(
        &format!("http://{addr}/api/auth/login"),
        json!({"account": ACCOUNT, "password": PASSWORD}),
    )
    .await;
    assert_eq!(resp.status(), 403);
    // The hooks' login was never consulted: no auth failure recorded, and
    // the (valid) credentials never minted a token.
    assert!(hooks.state().auth_failed.is_empty());
    assert!(hooks.state().logouts.is_empty());
}

#[tokio::test]
async fn healthz_reports_ok_and_uptime() {
    let hooks = Arc::new(MockWebHooks::new());
    // Publish a couple of values so the enriched fields are non-trivial.
    hooks.metrics_registry().ssh_connection_opened();
    hooks.metrics_registry().observe_loop_lag(9);
    hooks.metrics_registry().set_world_loop_iteration(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    );
    let addr = spawn_server(hooks, test_config()).await;

    let resp = client()
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    assert!(body["uptime_s"].is_u64());
    // Enriched liveness fields (PLAN observability): version, connections,
    // loop lag, store health.
    assert!(body["version"].is_string());
    assert_eq!(body["connections"], 1);
    assert_eq!(body["loop_lag_ms"], 9);
    assert_eq!(body["store_ok"], true);
}

#[tokio::test]
async fn metrics_endpoint_exposes_prometheus_text_without_secrets() {
    let hooks = Arc::new(MockWebHooks::new());
    let m = hooks.metrics_registry();
    m.ssh_connection_opened();
    m.web_connection_opened();
    m.command_processed();
    m.observe_loop_lag(3);
    m.publish_world(mush_net_web::WorldSample {
        sessions_current: 1,
        rooms: 4,
        players: 2,
        npc_active: 1,
        ..Default::default()
    });
    let addr = spawn_server(hooks, test_config()).await;

    let resp = client()
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(ct.starts_with("text/plain"), "content-type: {ct}");
    let body = resp.text().await.unwrap();
    // Prometheus shape.
    assert!(body.contains("# TYPE mush_connections gauge"));
    assert!(body.contains("mush_connections{transport=\"ssh\"} 1"));
    assert!(body.contains("mush_commands_total 1"));
    assert!(body.contains("mush_objects{kind=\"room\"} 4"));
    assert!(body.contains("mush_npc_active 1"));
    // NEVER any per-player / secret material. ("token" appears in aggregate
    // metric names like mush_llm_tokens_total, so the guard targets
    // secret-shaped label keys and credential material.)
    for forbidden in [
        "token=\"",
        "password",
        "Bearer",
        "account=\"",
        "player=\"",
        "ip=\"",
        "actor=\"",
        "Nick",
    ] {
        assert!(
            !body.contains(forbidden),
            "/metrics must not contain {forbidden:?}"
        );
    }
}

#[tokio::test]
async fn metrics_endpoint_off_when_disabled() {
    let hooks = Arc::new(MockWebHooks::new());
    let mut config = test_config();
    config.metrics_enabled = false;
    let addr = spawn_server(hooks, config).await;

    // Disabled → the route is not registered, so the SPA fallback answers
    // (404 when no embedded assets, never the metrics text).
    let resp = client()
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap_or_default();
    assert!(
        !body.contains("mush_connections"),
        "metrics must not be served when disabled"
    );
}

/// The SPA embed: tolerant of `web/dist` not being built (fresh clone), but
/// when it exists, `/`, deep links, and hashed-asset cache headers are all
/// exercised.
#[tokio::test]
async fn spa_embed_and_fallback_smoke() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks, test_config()).await;
    let c = client();

    // Unknown API paths must 404, never serve the SPA.
    let resp = c
        .get(format!("http://{addr}/api/no/such/route"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let index = c.get(format!("http://{addr}/")).send().await.unwrap();
    if index.status() == 404 {
        // web/dist not built in this checkout; nothing more to assert.
        return;
    }
    assert_eq!(index.status(), 200);
    let ct = index.headers()["content-type"].to_str().unwrap().to_owned();
    assert!(ct.starts_with("text/html"), "index content-type: {ct}");
    assert_eq!(index.headers()["cache-control"], "no-cache");

    // SPA deep link falls back to index.html.
    let deep = c
        .get(format!("http://{addr}/some/client/route"))
        .send()
        .await
        .unwrap();
    assert_eq!(deep.status(), 200);
    assert!(
        deep.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );

    // A hashed asset (found on disk, since the embed reads web/dist) gets
    // the immutable cache header.
    let assets_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist/assets");
    let Some(asset) = std::fs::read_dir(&assets_dir).ok().and_then(|rd| {
        rd.filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.ends_with(".js"))
    }) else {
        return;
    };
    let resp = c
        .get(format!("http://{addr}/assets/{asset}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        resp.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
}

/// LOW finding: every response carries a baseline security header set.
#[tokio::test]
async fn security_headers_are_present_on_every_response() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks, test_config()).await;

    for url in [
        format!("http://{addr}/healthz"),
        format!("http://{addr}/api/no/such/route"), // still hits the middleware stack (404)
    ] {
        let resp = client().get(&url).send().await.unwrap();
        let h = resp.headers();
        assert_eq!(h["x-content-type-options"], "nosniff", "{url}");
        assert_eq!(h["x-frame-options"], "DENY", "{url}");
        assert_eq!(h["referrer-policy"], "no-referrer", "{url}");
        let csp = h["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("default-src 'self'"), "{url}: {csp}");
        assert!(csp.contains("connect-src 'self' ws: wss:"), "{url}: {csp}");
        // Doc 21 cut G.3b: WebAssembly compiles (the cockpit picture), and
        // nothing else that evaluates does.
        assert!(
            csp.contains("script-src 'self' 'wasm-unsafe-eval';"),
            "{url}: {csp}"
        );
        assert!(!csp.contains("'unsafe-eval'"), "{url}: {csp}");
    }
}

/// MEDIUM-LOW finding: the auth rate limiter's key, when `trusted_proxies`
/// is set, must ignore `X-Forwarded-For` from a peer that is not itself a
/// trusted proxy — otherwise a direct attacker (not the proxy) could forge
/// distinct `X-Forwarded-For` values to spray requests across many buckets
/// and defeat the limiter entirely.
#[tokio::test]
async fn trusted_proxies_rate_limit_key_ignores_untrusted_peer_xff() {
    let hooks = Arc::new(MockWebHooks::new());
    // No proxy actually sits in front in this test — the point is that an
    // *untrusted* direct peer's forged header must not be trusted either.
    let config = WebConfig {
        trusted_proxies: vec!["10.0.0.1".parse().unwrap()],
        ..test_config()
    };
    let addr = spawn_server(hooks, config).await;
    let url = format!("http://{addr}/api/auth/login");
    let body = json!({"account": "ghost", "password": "x"});

    // Burst 5, exhausted by requests that all key on the real (untrusted)
    // peer IP despite each claiming a different forged X-Forwarded-For.
    for i in 0..5 {
        let resp = client()
            .post(&url)
            .header("x-forwarded-for", format!("10.9.9.{i}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401, "request {i} should pass the limiter");
    }
    let resp = client()
        .post(&url)
        .header("x-forwarded-for", "10.9.9.99")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        429,
        "a forged XFF from an untrusted peer must not dodge the limiter"
    );
}

#[tokio::test]
async fn rate_limit_disabled_when_zeroed() {
    let hooks = Arc::new(MockWebHooks::new());
    let config = WebConfig {
        auth_burst: 0,
        ..test_config()
    };
    let addr = spawn_server(hooks, config).await;
    let url = format!("http://{addr}/api/auth/login");
    for _ in 0..8 {
        let resp = post_json(&url, json!({"account": "ghost", "password": "x"})).await;
        assert_eq!(resp.status(), 401);
    }
}

#[tokio::test]
async fn auth_routes_fold_ipv6_slash_64_for_rate_limit() {
    let hooks = Arc::new(MockWebHooks::new());
    let config = WebConfig {
        trusted_proxies: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        ..test_config()
    };
    let addr = spawn_server(hooks, config).await;
    let url = format!("http://{addr}/api/auth/login");
    let body = json!({"account": "ghost", "password": "x"});

    // Five distinct host IDs in one IPv6 /64 share one governor bucket.
    for i in 1..=5 {
        let resp = client()
            .post(&url)
            .header("x-forwarded-for", format!("2001:db8:cafe:1::{i:x}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401, "request {i} should consume burst");
    }
    let resp = client()
        .post(&url)
        .header("x-forwarded-for", "2001:db8:cafe:1::ffff")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        429,
        "rotating host IDs inside the same /64 must share the auth rate limit"
    );

    // A separate /64 has its own bucket.
    let other = client()
        .post(&url)
        .header("x-forwarded-for", "2001:db8:cafe:2::1")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), 401);
}

#[tokio::test]
async fn auth_routes_enforce_4kib_body_limit() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/login");

    let huge_body = format!(
        "{{\"account\":\"nick\",\"password\":\"{}\"}}",
        "a".repeat(8_192)
    );
    let resp = client()
        .post(&url)
        .header("content-type", "application/json")
        .body(huge_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    assert!(hooks.state().auth_failed.is_empty());
}

#[tokio::test]
async fn blocked_ip_gets_403_before_json_parse_or_body_limit() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks
        .state()
        .blocked
        .insert(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/login");

    // Malformed JSON still gets 403 because is_ip_blocked runs before Json<T>.
    let resp = client()
        .post(&url)
        .header("content-type", "application/json")
        .body("{not valid json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);

    // Oversized body also gets 403 before body limit/parsing.
    let resp2 = client()
        .post(&url)
        .header("content-type", "application/json")
        .body("x".repeat(8_192))
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 403);
    assert!(hooks.state().auth_failed.is_empty());
}

#[tokio::test]
async fn login_rejects_oversized_credentials_without_calling_hook() {
    let hooks = Arc::new(MockWebHooks::new());
    // Set login_err = Banned (403) so hitting hooks.login would return 403,
    // proving oversized credentials are stopped before the hook runs.
    hooks.state().login_err = Some(LoginErr::Banned);
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let url = format!("http://{addr}/api/auth/login");

    let long_acct = post_json(
        &url,
        json!({"account": "a".repeat(65), "password": "secret"}),
    )
    .await;
    assert_eq!(long_acct.status(), 401);

    let long_pw = post_json(
        &url,
        json!({"account": "nick", "password": "p".repeat(129)}),
    )
    .await;
    assert_eq!(long_pw.status(), 401);
    assert_eq!(hooks.state().auth_failed.len(), 2);
}

#[tokio::test]
async fn register_rate_limited_maps_to_429() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().register_result = Some(Err(RegisterErr::RateLimited));
    let addr = spawn_server(hooks, test_config()).await;
    let resp = post_json(
        &format!("http://{addr}/api/auth/register"),
        json!({"account": "newbie", "password": "hunter22", "char_name": "Newbie"}),
    )
    .await;
    assert_eq!(resp.status(), 429);
}

#[tokio::test]
async fn slowloris_headers_time_out_and_close_connection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let hooks = Arc::new(MockWebHooks::new());
    let config = WebConfig {
        header_read_timeout_ms: 200,
        ..test_config()
    };
    let addr = spawn_server(hooks, config).await;

    // Case 1: partial headers without CRLFCRLF.
    let mut partial = TcpStream::connect(addr).await.unwrap();
    partial
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 64];
    let res = within("the Slowloris socket's close", partial.read(&mut buf)).await;
    assert!(
        matches!(res, Ok(0) | Err(_)),
        "expected EOF or reset on timed-out partial headers, got {res:?}"
    );

    // Case 2: 0-byte idle TCP connect.
    let mut idle = TcpStream::connect(addr).await.unwrap();
    let res2 = within("the 0-byte idle socket's close", idle.read(&mut buf)).await;
    assert!(
        matches!(res2, Ok(0) | Err(_)),
        "expected EOF or reset on 0-byte idle socket, got {res2:?}"
    );
}

#[tokio::test]
async fn global_tcp_connection_cap_drops_excess_sockets() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let hooks = Arc::new(MockWebHooks::new());
    hooks.metrics_registry().set_world_loop_iteration(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    );
    let config = WebConfig {
        max_connections: 2,
        header_read_timeout_ms: 5_000,
        ..test_config()
    };
    let addr = spawn_server(hooks, config).await;

    // Hold both connection slots with two open HTTP sockets.
    let mut held1 = TcpStream::connect(addr).await.unwrap();
    let mut held2 = TcpStream::connect(addr).await.unwrap();
    held1.write_all(b"GET ").await.unwrap();
    held2.write_all(b"GET ").await.unwrap();
    // No wait for the permits. One accept loop takes the listener's sockets in
    // the order their handshakes completed and acquires a socket's permit
    // before it accepts the next, and `connect` returns once the handshake is
    // done. So the third socket below is accepted after both held ones hold
    // their permits.

    // Third socket must be dropped immediately without an HTTP response.
    let mut excess = TcpStream::connect(addr).await.unwrap();
    let _ = excess
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await;
    let mut buf = [0u8; 64];
    let read_res = within("the excess socket's close", excess.read(&mut buf)).await;
    assert!(
        matches!(read_res, Ok(0) | Err(_)),
        "excess socket must not receive an HTTP response while cap is saturated: {read_res:?}"
    );

    // Releasing one held socket restores capacity.
    drop(held1);
    until_served(
        addr,
        "a fresh connection served once a held socket is dropped",
    )
    .await;
    drop(held2);
}

#[tokio::test]
async fn request_timeout_returns_408_without_cutting_ws() {
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().login_delay = Some(Duration::from_millis(400));
    let config = WebConfig {
        request_timeout_ms: 150,
        // The socket's first ping comes at twice the request timeout: the
        // mark that it outlived it.
        ping_interval_ms: 300,
        ..test_config()
    };
    assert!(config.ping_interval_ms > config.request_timeout_ms);
    let addr = spawn_server(hooks, config).await;

    // Slow non-upgrade HTTP request times out with 408.
    let resp = post_json(
        &format!("http://{addr}/api/auth/login"),
        json!({"account": ACCOUNT, "password": PASSWORD}),
    )
    .await;
    assert_eq!(resp.status(), 408);

    // A WebSocket on `/ws` stays alive well past `request_timeout_ms`.
    let mut ws_req = format!("ws://{addr}/ws?char={}", common::CHAR_KEY)
        .into_client_request()
        .unwrap();
    ws_req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rustmush.v1.{TOKEN}")).unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_req).await.unwrap();
    let ping = first_ping(&mut ws).await;
    assert_eq!(
        ping.seq, 0,
        "WS connection must survive past request_timeout_ms"
    );
}

#[tokio::test]
async fn p2_edge_2_per_ip_cap_blocked_all_routes_and_keepalive_limits_preserve_ws() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let hooks = Arc::new(MockWebHooks::new());
    hooks.metrics_registry().set_world_loop_iteration(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    );
    let config = WebConfig {
        max_connections: 16,
        max_connections_per_ip: 2,
        max_keepalive_requests: 2,
        max_http_conn_age_ms: 400,
        header_read_timeout_ms: 5_000,
        metrics_enabled: true,
        // The upgraded socket's first ping comes half again past the HTTP
        // connection age: the mark that the age did not cut it.
        ping_interval_ms: 600,
        ..test_config()
    };
    assert!(config.ping_interval_ms > config.max_http_conn_age_ms);
    let addr = spawn_server(hooks.clone(), config).await;

    // 1. Per-IP cap (2) drops 3rd socket even though global max_connections is 16.
    let mut held1 = TcpStream::connect(addr).await.unwrap();
    let mut held2 = TcpStream::connect(addr).await.unwrap();
    held1.write_all(b"GET ").await.unwrap();
    held2.write_all(b"GET ").await.unwrap();
    // No wait for the slots: the accept loop takes sockets in handshake order
    // (as in `global_tcp_connection_cap_drops_excess_sockets`).

    let mut excess = TcpStream::connect(addr).await.unwrap();
    let _ = excess
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await;
    let mut buf = [0u8; 64];
    let res = within("the 3rd socket's close", excess.read(&mut buf)).await;
    assert!(
        matches!(res, Ok(0) | Err(_)),
        "per-IP cap must drop 3rd socket before HTTP response: {res:?}"
    );
    drop((held1, held2));
    until_served(
        addr,
        "a fresh connection served once both held sockets are dropped",
    )
    .await;

    // 2. max_keepalive_requests (2) sends Connection: close on the 2nd response
    //    and closes the socket rather than allowing indefinite keep-alive pinning.
    let mut ka = TcpStream::connect(addr).await.unwrap();
    for i in 0..2 {
        ka.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n")
            .await
            .unwrap();
        let mut resp_bytes = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = within("a healthz response", ka.read(&mut chunk))
                .await
                .unwrap();
            assert!(n > 0, "unexpected EOF on request {i}");
            resp_bytes.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&resp_bytes);
            if text.contains("\r\n\r\n") && text.contains("\"ok\"") {
                if i == 1 {
                    assert!(
                        text.to_ascii_lowercase().contains("connection: close"),
                        "2nd keep-alive response must include Connection: close: {text}"
                    );
                }
                break;
            }
        }
    }
    let eof = within(
        "the keep-alive socket's close after max_keepalive_requests",
        ka.read(&mut buf),
    )
    .await;
    assert!(matches!(eof, Ok(0) | Err(_)), "expected EOF: {eof:?}");

    // 3. WebSocket upgrade survives well past max_http_conn_age_ms (400ms).
    let mut ws_req = format!("ws://{addr}/ws?char={}", common::CHAR_KEY)
        .into_client_request()
        .unwrap();
    ws_req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rustmush.v1.{TOKEN}")).unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_req).await.unwrap();
    let ping = first_ping(&mut ws).await;
    assert_eq!(
        ping.seq, 0,
        "upgraded WebSocket must not be cut by max_http_conn_age_ms"
    );
    drop(ws);
    until_served(addr, "a fresh connection served once the socket is dropped").await;

    // 4. is_ip_blocked is enforced across ALL routes, not just /api/auth/*.
    hooks
        .state()
        .blocked
        .insert(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    for path in ["/healthz", "/metrics", "/"] {
        let resp = client()
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403, "blocked IP must get 403 on {path}");
    }
}

#[tokio::test]
async fn client_downloads_matches_server_commit_and_falls_back_to_latest() {
    let dir = std::env::temp_dir().join(format!(
        "rustmush-web-http-client-dl-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("RustMUSH_TacSim-windows-x64-abcdef123.zip"),
        b"exact-match-zip",
    )
    .unwrap();
    std::fs::write(
        dir.join("RustMUSH_TacSim-windows-x64-abcdef123.zip.json"),
        r#"{"target":"windows-x64","commit":"abcdef1234567890abcdef1234567890abcdef12","short_commit":"abcdef123","built_at_utc":"2026-11-20T08:00:00Z"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("RustMUSH_TacSim-windows-x64-999999999.zip"),
        b"newer-other-commit-zip",
    )
    .unwrap();
    std::fs::write(
        dir.join("RustMUSH_TacSim-windows-x64-999999999.zip.json"),
        r#"{"target":"windows-x64","commit":"9999999999999999999999999999999999999999","short_commit":"999999999","built_at_utc":"2026-11-22T08:00:00Z"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("RustMUSH_TacSim-linux-x64-888888888.zip"),
        b"linux-latest",
    )
    .unwrap();

    let hooks = Arc::new(MockWebHooks::new());
    let cfg = WebConfig {
        client_downloads_dir: Some(dir.clone()),
        server_commit: Some("abcdef1234567890abcdef1234567890abcdef12".into()),
        ..test_config()
    };
    let addr = spawn_server(hooks, cfg).await;

    let resp = client()
        .get(format!("http://{addr}/api/client-downloads"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );
    let body: Value = resp.json().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(body["server_commit"], "abcdef123");
    let clients = body["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 2);
    assert_eq!(clients[0]["target"], "windows-x64");
    assert_eq!(clients[0]["exact_match"], true);
    assert_eq!(
        clients[0]["url"],
        "/downloads/clients/RustMUSH_TacSim-windows-x64-abcdef123.zip"
    );
    assert_eq!(clients[1]["target"], "linux-x64");
    assert_eq!(clients[1]["exact_match"], false);
    assert_eq!(
        clients[1]["url"],
        "/downloads/clients/RustMUSH_TacSim-linux-x64-888888888.zip"
    );
}
