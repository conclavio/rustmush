//! Shared test fixture: [`MockWebHooks`] plus a real bound server.
//!
//! Every test spawns its own server on an ephemeral port (fresh rate-limit
//! buckets, fresh mock state) and talks real HTTP/WS to it — the router is
//! served exactly the way `mushd` serves it (`with_connect_info`), so the
//! governor's peer-IP key extraction and the IpGuard path are exercised for
//! real.

#![allow(dead_code)] // two test binaries share this; each uses a subset

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_net_web::{
    AttachErr, CharSummary, LoginErr, LoginOk, NewWebConn, RegisterErr, ResumeErr, ResumeOk,
    SessionStart, TokenIdentity, TokenVerdict, WebConfig, WebHooks,
};
use mush_proto::{Envelope, OutputEvent, ProjectedCommitToken, Query, RichText};
use tokio::sync::mpsc;

/// The fixture account / token the default mock knows about.
pub const ACCOUNT: &str = "nick";
pub const PASSWORD: &str = "pw";
pub const TOKEN: &str = "tok-nick";
pub const CHAR_KEY: u64 = 7;

#[derive(Default)]
pub struct MockState {
    // -- configuration --
    pub accounts: HashMap<String, String>,
    pub tokens: HashMap<String, TokenIdentity>,
    pub blocked: HashSet<IpAddr>,
    /// Addresses the mock reports as inside a refused-token cool-off.
    pub token_throttled: HashSet<IpAddr>,
    /// The token store cannot answer: every `validate_token` comes back
    /// [`TokenVerdict::Unavailable`], whatever token is presented (a Redis
    /// restart, a reconnecting pool).
    pub token_store_down: bool,
    pub login_err: Option<LoginErr>,
    pub register_result: Option<Result<LoginOk, RegisterErr>>,
    pub connect_err: Option<AttachErr>,
    pub resume_result: Option<Result<ResumeOk, ResumeErr>>,
    // -- recordings --
    pub lines: Vec<(u64, String)>,
    pub requests: Vec<(u64, u64, Query)>,
    pub acknowledgements: Vec<(u64, ProjectedCommitToken)>,
    pub disconnects: Vec<u64>,
    pub auth_failed: Vec<IpAddr>,
    /// Refused-token counts, kept apart from `auth_failed` exactly as the
    /// guard keeps the two counters apart.
    pub token_refused: Vec<IpAddr>,
    pub logouts: Vec<String>,
    pub connects: Vec<(u64, String, u64)>,
    pub resumes: Vec<(u64, String, u64, u64)>,
    /// Captured per-connection envelope senders (the game side's handle).
    pub conns: HashMap<u64, mpsc::Sender<Arc<Envelope>>>,
    /// Per-IP concurrent WS connection cap for `connection_opened`/
    /// `connection_closed` (`None` = unlimited, the trait default).
    pub conn_cap: Option<usize>,
    /// Live count against `conn_cap`, incremented/decremented by
    /// `connection_opened`/`connection_closed`.
    pub live_conns: usize,
    pub conn_opens: Vec<IpAddr>,
    pub conn_closes: Vec<IpAddr>,
}

pub struct MockWebHooks {
    state: Mutex<MockState>,
    metrics: Arc<mush_net_web::Metrics>,
}

impl MockWebHooks {
    /// A mock that knows `nick`/`pw`, whose live token is `tok-nick` for
    /// one character (`key 7, "Nick"`).
    pub fn new() -> Self {
        let mut st = MockState::default();
        st.accounts.insert(ACCOUNT.into(), PASSWORD.into());
        st.tokens.insert(
            TOKEN.into(),
            TokenIdentity {
                account: ACCOUNT.into(),
                characters: vec![CharSummary {
                    key: CHAR_KEY,
                    name: "Nick".into(),
                }],
            },
        );
        Self {
            state: Mutex::new(st),
            metrics: Arc::new(mush_net_web::Metrics::new()),
        }
    }

    /// The shared metrics registry `/healthz` and `/metrics` read.
    pub fn metrics_registry(&self) -> Arc<mush_net_web::Metrics> {
        Arc::clone(&self.metrics)
    }

    pub fn login_ok(&self) -> LoginOk {
        LoginOk {
            token: TOKEN.into(),
            account: ACCOUNT.into(),
            characters: vec![CharSummary {
                key: CHAR_KEY,
                name: "Nick".into(),
            }],
        }
    }

    pub fn state(&self) -> MutexGuard<'_, MockState> {
        self.state.lock().unwrap()
    }

    /// Polls until `pred` holds (10 ms cadence, 3 s deadline).
    pub async fn wait_for(&self, what: &str, pred: impl Fn(&MockState) -> bool) {
        for _ in 0..300 {
            if pred(&self.state()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for: {what}");
    }
}

impl WebHooks for MockWebHooks {
    fn login<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
        _ip: IpAddr,
    ) -> BoxFuture<'a, Result<LoginOk, LoginErr>> {
        Box::pin(async move {
            let st = self.state();
            if let Some(err) = st.login_err {
                return Err(err);
            }
            match st.accounts.get(account) {
                Some(pw) if pw == password => Ok(self.login_ok()),
                _ => Err(LoginErr::BadCreds),
            }
        })
    }

    fn register<'a>(
        &'a self,
        _account: &'a str,
        _password: &'a str,
        _char_name: &'a str,
        _ip: IpAddr,
    ) -> BoxFuture<'a, Result<LoginOk, RegisterErr>> {
        Box::pin(async move {
            self.state()
                .register_result
                .clone()
                .unwrap_or(Err(RegisterErr::Taken))
        })
    }

    fn logout<'a>(&'a self, token: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.state().logouts.push(token.to_string());
        })
    }

    fn validate_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, TokenVerdict> {
        Box::pin(async move {
            let st = self.state();
            if st.token_store_down {
                return TokenVerdict::Unavailable;
            }
            match st.tokens.get(token).cloned() {
                Some(identity) => TokenVerdict::Valid(identity),
                None => TokenVerdict::Refused,
            }
        })
    }

    fn connect_character<'a>(
        &'a self,
        conn: NewWebConn,
        token: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<SessionStart, AttachErr>> {
        Box::pin(async move {
            let mut st = self.state();
            if let Some(err) = st.connect_err.clone() {
                return Err(err);
            }
            st.connects.push((conn.id, token.to_string(), character));
            st.conns.insert(conn.id, conn.tx);
            Ok(SessionStart { resume_from: 0 })
        })
    }

    fn resume<'a>(
        &'a self,
        conn: NewWebConn,
        token: &'a str,
        character: u64,
        last_seq: u64,
    ) -> BoxFuture<'a, Result<ResumeOk, ResumeErr>> {
        Box::pin(async move {
            let mut st = self.state();
            st.resumes
                .push((conn.id, token.to_string(), character, last_seq));
            match st.resume_result.clone() {
                Some(Ok(ok)) => {
                    st.conns.insert(conn.id, conn.tx);
                    Ok(ok)
                }
                Some(Err(err)) => Err(err),
                None => Err(ResumeErr::BadSession),
            }
        })
    }

    fn line(&self, conn: u64, line: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.state().lines.push((conn, line));
        })
    }

    fn request(&self, conn: u64, id: u64, q: Query) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.state().requests.push((conn, id, q));
        })
    }

    fn acknowledge(&self, conn: u64, commit: ProjectedCommitToken) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.state().acknowledgements.push((conn, commit));
        })
    }

    fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.state().disconnects.push(conn);
        })
    }

    fn is_ip_blocked(&self, addr: IpAddr) -> bool {
        self.state().blocked.contains(&addr)
    }

    fn auth_failed(&self, addr: IpAddr) {
        self.state().auth_failed.push(addr);
    }

    fn token_refused(&self, addr: IpAddr) {
        self.state().token_refused.push(addr);
    }

    fn is_token_throttled(&self, addr: IpAddr) -> bool {
        self.state().token_throttled.contains(&addr)
    }

    fn connection_opened(&self, addr: IpAddr) -> bool {
        let mut st = self.state();
        st.conn_opens.push(addr);
        if st.conn_cap.is_some_and(|cap| st.live_conns >= cap) {
            return false;
        }
        st.live_conns += 1;
        true
    }

    fn connection_closed(&self, addr: IpAddr) {
        let mut st = self.state();
        st.conn_closes.push(addr);
        st.live_conns = st.live_conns.saturating_sub(1);
    }

    fn metrics(&self) -> Arc<mush_net_web::Metrics> {
        Arc::clone(&self.metrics)
    }
}

/// A stamped envelope as the session layer would emit (for replay / live
/// injection).
pub fn sys_env(seq: u64, text: &str) -> Arc<Envelope> {
    Arc::new(Envelope::new(
        seq,
        1_756_300_000_000 + seq as i64,
        OutputEvent::Sys(RichText::plain(text)),
    ))
}

/// Config tuned for tests: fixture features/panels, fast keepalive knobs
/// left at their defaults unless a test overrides them.
pub fn test_config() -> WebConfig {
    WebConfig {
        features: vec!["mail".into(), "channels".into()],
        panels: vec!["wod.sheet".into()],
        // The HTTP tests exercise `/metrics`; it is opt-in (default off), so
        // enable it here. `metrics_endpoint_off_when_disabled` flips it back.
        metrics_enabled: true,
        ..WebConfig::default()
    }
}

/// Binds an ephemeral port and serves the router exactly like `mushd` does.
pub async fn spawn_server(hooks: Arc<MockWebHooks>, config: WebConfig) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = mush_net_web::router(hooks, config);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}
