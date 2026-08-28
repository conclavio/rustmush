//! Route tests over real HTTP: auth flow, rate limiting, IpGuard, healthz,
//! and the embedded-SPA fallback.

mod common;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use common::{ACCOUNT, MockWebHooks, PASSWORD, TOKEN, spawn_server, test_config};
use mush_net_web::{LoginErr, LoginOk, RegisterErr, WebConfig};
use serde_json::{Value, json};

fn client() -> reqwest::Client {
    reqwest::Client::new()
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
