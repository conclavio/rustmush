//! `/api/auth/*` — login, register, logout.
//!
//! All three routes sit behind the tower_governor tight bucket (see
//! `app::rate_limit`) and check [`crate::WebHooks::is_ip_blocked`] first. Bad
//! credentials answer a **uniform 401** — unknown account and wrong
//! password are deliberately indistinguishable — and count one
//! [`crate::WebHooks::auth_failed`] against the address. Passwords and tokens are
//! never logged (the trace layer's span carries only method + path).

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::hooks::{CharSummary, LoginErr, LoginOk, RegisterErr};
use crate::ip::client_ip;

#[derive(Deserialize)]
pub(crate) struct LoginReq {
    account: String,
    password: String,
}

#[derive(Deserialize)]
pub(crate) struct RegisterReq {
    account: String,
    password: String,
    char_name: String,
}

#[derive(Serialize)]
struct LoginResp<'a> {
    token: &'a str,
    account: &'a str,
    characters: &'a [CharSummary],
}

/// The single 401 body for every credential failure (uniform on purpose).
const BAD_CREDS: &str = "invalid account or password";

fn err_json(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn login_ok(ok: &LoginOk) -> Response {
    Json(LoginResp {
        token: &ok.token,
        account: &ok.account,
        characters: &ok.characters,
    })
    .into_response()
}

/// `POST /api/auth/login` `{account, password}` →
/// `{token, account, characters: [{key, name}]}`.
pub(crate) async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Response {
    let ip = client_ip(peer.ip(), &headers, &state.config.trusted_proxies);
    if state.hooks.is_ip_blocked(ip) {
        return err_json(StatusCode::FORBIDDEN, "forbidden");
    }
    match state.hooks.login(&req.account, &req.password, ip).await {
        Ok(ok) => login_ok(&ok),
        Err(LoginErr::BadCreds) => {
            state.hooks.auth_failed(ip);
            err_json(StatusCode::UNAUTHORIZED, BAD_CREDS)
        }
        Err(LoginErr::Banned) => err_json(StatusCode::FORBIDDEN, "forbidden"),
        Err(LoginErr::RateLimited) => err_json(StatusCode::TOO_MANY_REQUESTS, "slow down"),
    }
}

/// `POST /api/auth/register` `{account, password, char_name}` — creates the
/// account plus its first character and logs it in (same response shape as
/// login).
pub(crate) async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Response {
    let ip = client_ip(peer.ip(), &headers, &state.config.trusted_proxies);
    if state.hooks.is_ip_blocked(ip) {
        return err_json(StatusCode::FORBIDDEN, "forbidden");
    }
    match state
        .hooks
        .register(&req.account, &req.password, &req.char_name, ip)
        .await
    {
        Ok(ok) => login_ok(&ok),
        Err(RegisterErr::Taken) => err_json(StatusCode::CONFLICT, "account name taken"),
        Err(RegisterErr::BadName(msg)) => err_json(StatusCode::UNPROCESSABLE_ENTITY, &msg),
        Err(RegisterErr::Weak) => err_json(StatusCode::UNPROCESSABLE_ENTITY, "password too weak"),
        Err(RegisterErr::Banned) => err_json(StatusCode::FORBIDDEN, "forbidden"),
    }
}

/// `POST /api/auth/logout` with `Authorization: Bearer <token>` — revokes
/// the token. 204 even for unknown tokens (idempotent).
pub(crate) async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let ip = client_ip(peer.ip(), &headers, &state.config.trusted_proxies);
    if state.hooks.is_ip_blocked(ip) {
        return err_json(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(token) = bearer_token(&headers) else {
        return err_json(StatusCode::BAD_REQUEST, "missing bearer token");
    };
    state.hooks.logout(token).await;
    StatusCode::NO_CONTENT.into_response()
}

/// Extracts the token from `Authorization: Bearer <token>`.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn bearer_token_parses() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc123"),
        );
        assert_eq!(bearer_token(&h), Some("abc123"));
    }

    #[test]
    fn bearer_token_rejects_missing_or_empty() {
        assert_eq!(bearer_token(&HeaderMap::new()), None);
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer "));
        assert_eq!(bearer_token(&h), None);
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic xx"));
        assert_eq!(bearer_token(&h), None);
    }
}
