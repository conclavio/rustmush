//! The end-to-end rig: a doorway on loopback TCP, an instance-side trusted
//! ingress reached in process, test hooks behind the ingress, and client
//! helpers.
//!
//! The rig waits through the story kit (doc 51 unit FL.9): every condition
//! through its one wait loop, every event under its one step limit
//! ([`STEP_TIMEOUT`]), and its data directory from [`scratch_dir`], which no
//! clock names. The stories wait for what the doorway does, not for a span:
//! [`Rig::stopped_serving`] for a drain's closed listener, and
//! [`Rig::deadline_mark`] for "the deadline has run out".

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    dead_code
)]

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_net_ssh::{AuthResult, BlockReason, CharSummary, CreateErr, NewConn, SshHooks};
use mush_proto::{ClientCaps, OutputEvent, RichText};
use mush_sshdoorway::Doorway;
use mush_sshdoorway::config::{Config, ServiceCfg};
use mush_sshdoorway::upstream::{BoxStream, Connector};
pub use mush_storykit::{STEP_TIMEOUT, round_trip, scratch_dir, wait_until, within};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use russh::{Channel, ChannelMsg, client};

// ---- the instance side ----------------------------------------------------

#[derive(Default)]
pub struct State {
    pub opened: Vec<IpAddr>,
    pub closed: Vec<IpAddr>,
    pub failures: Vec<IpAddr>,
    pub connected: Vec<(String, u64)>,
    pub lines: Vec<String>,
    pub widths: Vec<u16>,
    pub conns: Vec<NewConn>,
    pub disconnected: Vec<u64>,
}

#[derive(Default)]
pub struct Hooks {
    pub leases: mush_net_ssh::resume::Registry,
    pub blocked: bool,
    pub state: Mutex<State>,
}

pub fn test_key(seed: u8) -> PrivateKey {
    PrivateKey::from(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(
        &[seed; 32],
    ))
}

pub fn with_hash(key: PrivateKey) -> PrivateKeyWithHashAlg {
    PrivateKeyWithHashAlg::new(Arc::new(key), None)
}

fn key_material(openssh: &str) -> String {
    openssh
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

impl Hooks {
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
    pub fn opened(&self) -> Vec<IpAddr> {
        self.st().opened.clone()
    }
    pub fn open_now(&self) -> usize {
        let st = self.st();
        st.opened.len().saturating_sub(st.closed.len())
    }
    pub fn failures(&self) -> Vec<IpAddr> {
        self.st().failures.clone()
    }
    pub fn connected(&self) -> Vec<(String, u64)> {
        self.st().connected.clone()
    }
    pub fn lines(&self) -> Vec<String> {
        self.st().lines.clone()
    }
    pub fn widths(&self) -> Vec<u16> {
        self.st().widths.clone()
    }
    /// Queue one event for the `nth` connected session (in connect order),
    /// as the game would. `false` when there is none or its queue is closed.
    pub fn send(&self, nth: usize, event: OutputEvent) -> bool {
        self.st()
            .conns
            .get(nth)
            .is_some_and(|conn| conn.tx.try_send(Arc::new(event)).is_ok())
    }
    /// `@boot` every connected session (drop its output queue).
    pub fn boot_all(&self) {
        self.st().conns.clear();
    }
    /// Push `count` lines of `size` bytes at the first connected session.
    pub fn flood(&self, size: usize, count: usize) -> tokio::task::JoinHandle<()> {
        let tx = self.st().conns.first().map(|c| c.tx.clone());
        tokio::spawn(async move {
            let Some(tx) = tx else { return };
            let line = "x".repeat(size);
            for _ in 0..count {
                let ev = Arc::new(OutputEvent::Sys(RichText::plain(line.clone())));
                if tx.send(ev).await.is_err() {
                    return;
                }
            }
        })
    }
}

impl SshHooks for Hooks {
    fn auth_account<'a>(&'a self, a: &'a str, p: &'a str) -> BoxFuture<'a, AuthResult> {
        Box::pin(async move {
            if a == "faenix" && p == "pw" {
                AuthResult::Ok {
                    account: "faenix".into(),
                    characters: vec![CharSummary {
                        key: 1,
                        name: "Kira".into(),
                    }],
                }
            } else {
                AuthResult::BadCreds
            }
        })
    }
    fn auth_account_key<'a>(&'a self, a: &'a str, k: &'a str) -> BoxFuture<'a, AuthResult> {
        Box::pin(async move {
            let registered = key_material(&test_key(1).public_key().to_openssh().unwrap());
            if a == "faenix" && key_material(k) == registered {
                AuthResult::Ok {
                    account: "faenix".into(),
                    characters: vec![CharSummary {
                        key: 1,
                        name: "Kira".into(),
                    }],
                }
            } else {
                AuthResult::BadCreds
            }
        })
    }
    fn create_account<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: IpAddr,
    ) -> BoxFuture<'a, Result<(), CreateErr>> {
        Box::pin(async { Err(CreateErr::Weak) })
    }
    fn connect_character<'a>(
        &'a self,
        conn: NewConn,
        account: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<(), String>> {
        let mut st = self.st();
        st.connected.push((account.to_owned(), character));
        st.conns.push(conn);
        Box::pin(async { Ok(()) })
    }
    fn line(&self, _: u64, line: String) -> BoxFuture<'_, ()> {
        self.st().lines.push(line);
        Box::pin(async {})
    }
    fn cockpit_issue<'a>(
        &'a self,
        id: u64,
        pane: &'a str,
        account: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<(mush_net_ssh::resume::Ticket, u32), mush_net_ssh::resume::Failure>>
    {
        Box::pin(async move {
            self.leases.issue(
                id,
                pane,
                account,
                character,
                mush_session::SessId(19),
                Duration::from_secs(60),
            )
        })
    }
    fn cockpit_redeem<'a>(
        &'a self,
        conn: NewConn,
        pane: &'a str,
        account: &'a str,
        selected: Option<u64>,
        ticket: &'a mush_net_ssh::resume::Ticket,
    ) -> BoxFuture<'a, Result<mush_net_ssh::resume::Redeemed, mush_net_ssh::resume::Failure>> {
        Box::pin(async move {
            let claim = self
                .leases
                .reserve(conn.id, pane, account, selected, ticket)?;
            let character = claim.character();
            let view = claim.view();
            let (ticket, grace) = claim.commit()?;
            self.st().conns.push(conn);
            Ok((character, ticket, grace, view))
        })
    }
    fn cockpit_view(
        &self,
        id: u64,
        account: &str,
        pane: &str,
        view: mush_net_ssh::resume::ViewPrefs,
    ) {
        self.leases.update_view(id, account, pane, view);
    }
    fn cockpit_close(&self, id: u64) {
        self.leases.detach(id, false);
    }
    fn disconnected(&self, id: u64) -> BoxFuture<'_, ()> {
        self.leases.detach(id, true);
        self.st().disconnected.push(id);

        Box::pin(async {})
    }
    fn is_ip_blocked(&self, _: IpAddr) -> bool {
        self.blocked
    }
    fn block_reason(&self, _: IpAddr) -> Option<BlockReason> {
        self.blocked.then_some(BlockReason::Siteban)
    }
    fn auth_failed(&self, addr: IpAddr) {
        self.st().failures.push(addr);
    }
    fn connection_opened(&self, addr: IpAddr) -> bool {
        self.st().opened.push(addr);
        true
    }
    fn connection_closed(&self, addr: IpAddr) {
        self.st().closed.push(addr);
    }
    fn caps_changed(&self, _: u64, caps: ClientCaps) {
        self.st().widths.push(caps.width);
    }
}

/// Dials the in-process ingress over a duplex stream.
pub struct InProc {
    ingress: mush_net_ssh::TrustedIngress,
    fail: AtomicBool,
}

impl InProc {
    pub fn fail_next(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }
}

impl Connector for InProc {
    fn connect<'a>(&'a self, _: &'a ServiceCfg) -> BoxFuture<'a, std::io::Result<BoxStream>> {
        Box::pin(async move {
            if self.fail.swap(false, Ordering::SeqCst) {
                return Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
            }
            let (doorway, instance) = tokio::io::duplex(256 * 1024);
            let ingress = self.ingress.clone();
            tokio::spawn(async move { ingress.serve(instance).await });
            Ok(Box::new(doorway) as BoxStream)
        })
    }
}

// ---- the rig ---------------------------------------------------------------

pub struct Rig {
    pub door: Arc<Doorway>,
    pub hooks: Arc<Hooks>,
    pub connector: Arc<InProc>,
    pub addr: SocketAddr,
    banners: Arc<Mutex<Vec<String>>>,
    dir: std::path::PathBuf,
    /// The accept loop's task: it ends when a drain begins, dropping the
    /// listener with it.
    serving: Mutex<Option<tokio::task::JoinHandle<std::io::Result<()>>>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

impl Rig {
    pub async fn start(hooks: Hooks) -> Rig {
        Rig::start_with(hooks, |_| {}).await
    }

    pub async fn start_with(hooks: Hooks, tweak: impl FnOnce(&mut Config)) -> Rig {
        let dir = scratch_dir("sshdoorway-e2e", "rig").unwrap();
        let text = format!(
            r#"
[listen]
addrs = ["127.0.0.1:2222"]
host_key = {host_key:?}

[limits]
# The rig's clients all come from loopback, which the default exempts from
# the per-IP caps; the cap tests need to be able to hit them.
exempt_nets = []

[menu]
title = "test menu"
host = "test.host"

[[service]]
name = "dev1"
title = "Development server 1"
socket = "unused"

[[service]]
name = "hidden"
title = "Hidden"
socket = "unused"
listed = false
"#,
            host_key = dir.join("doorway_host").display().to_string()
        );
        let mut cfg = Config::parse(&text).unwrap();
        tweak(&mut cfg);
        cfg.validate().unwrap();
        let hooks = Arc::new(hooks);
        let ingress = mush_net_ssh::TrustedIngress::new(
            &dir.join("instance_host"),
            "rustmush test banner",
            Arc::clone(&hooks) as Arc<dyn SshHooks>,
        )
        .unwrap();
        let connector = Arc::new(InProc {
            ingress,
            fail: AtomicBool::new(false),
        });
        let door = Doorway::new(cfg, Arc::clone(&connector) as Arc<dyn Connector>).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let d = Arc::clone(&door);
        let serving = tokio::spawn(async move { d.serve(listener).await });
        Rig {
            door,
            hooks,
            connector,
            addr,
            banners: Arc::default(),
            dir,
            serving: Mutex::new(Some(serving)),
        }
    }

    /// **The accept loop has returned** (doc 51 §3.5): a drain has begun and
    /// the listener is dropped, so a new connection is refused from here on.
    /// The event, from the rig, in place of a sleep after `drain.begin()`.
    pub async fn stopped_serving(&self) {
        let serving = self
            .serving
            .lock()
            .unwrap()
            .take()
            .expect("the accept loop is awaited once");
        within("the accept loop's return", serving)
            .await
            .expect("the accept loop did not panic")
            .expect("the accept loop ends cleanly");
    }

    /// **The mark that the auth deadline has run out** (doc 51 §3.3) for
    /// everything the doorway started a deadline for before this call: a
    /// connection accepted, or a session's last channel retired, which starts
    /// the same span of idle grace.
    ///
    /// A client connects now and never authenticates. The doorway cuts it at
    /// its auth deadline, which began after theirs, so when it is cut theirs
    /// has run out too. A story asserts "still open" then, where it slept past
    /// the deadline. A packet whose handling starts a deadline has to have
    /// been handled first: [`round_trip`] on its connection says so.
    pub async fn deadline_mark(&self) {
        let mark = self.client().await;
        wait_until(
            STEP_TIMEOUT,
            "the mark's cut at its auth deadline",
            async || mark.is_closed(),
        )
        .await;
    }

    pub async fn direct_listener(&self) -> SocketAddr {
        let cfg = mush_net_ssh::SshConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            host_key_path: self.dir.join("instance_host"),
            banner: "rustmush test banner".into(),
            auth_deadline: mush_net_ssh::DEFAULT_AUTH_DEADLINE,
        };
        let bound =
            mush_net_ssh::SshServer::bind(cfg, Arc::clone(&self.hooks) as Arc<dyn SshHooks>)
                .await
                .unwrap();
        let addr = bound.local_addr();
        tokio::spawn(bound.run());
        addr
    }
    pub async fn droppable_client(
        &self,
        addr: SocketAddr,
    ) -> (client::Handle<Cli>, std::net::TcpStream) {
        let stream = tokio::net::TcpStream::connect(addr)
            .await
            .unwrap()
            .into_std()
            .unwrap();
        let cut = stream.try_clone().unwrap();
        let client = client::connect_stream(
            Arc::new(client::Config::default()),
            tokio::net::TcpStream::from_std(stream).unwrap(),
            Cli {
                banners: self.banners.clone(),
            },
        )
        .await
        .unwrap();
        (client, cut)
    }
    pub async fn try_client(&self) -> Result<client::Handle<Cli>, String> {
        let cfg = Arc::new(client::Config::default());
        let cli = Cli {
            banners: Arc::clone(&self.banners),
        };
        match mush_storykit::at_most(STEP_TIMEOUT, client::connect(cfg, self.addr, cli)).await {
            Some(Ok(h)) => Ok(h),
            Some(Err(e)) => Err(e.to_string()),
            None => Err("timed out".into()),
        }
    }

    pub async fn client(&self) -> client::Handle<Cli> {
        self.try_client().await.unwrap()
    }

    pub fn banners(&self) -> Vec<String> {
        self.banners.lock().unwrap().clone()
    }
}

pub struct Cli {
    banners: Arc<Mutex<Vec<String>>>,
}

impl client::Handler for Cli {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.banners.lock().unwrap().push(banner.to_owned());
        Ok(())
    }
}

// ---- client helpers ----------------------------------------------------------

pub async fn next_reply(ch: &mut Channel<client::Msg>) -> bool {
    loop {
        match within("a channel request's reply", ch.wait()).await {
            Some(ChannelMsg::Success) => return true,
            Some(ChannelMsg::Failure) | None => return false,
            Some(_) => {}
        }
    }
}

pub async fn shell(h: &client::Handle<Cli>) -> Channel<client::Msg> {
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_pty(true, "xterm-256color", 120, 40, 1200, 800, &[])
        .await
        .unwrap();
    assert!(next_reply(&mut ch).await, "pty-req refused");
    ch.request_shell(true).await.unwrap();
    assert!(next_reply(&mut ch).await, "shell refused");
    ch
}

pub async fn read_until(ch: &mut Channel<client::Msg>, needle: &str) -> String {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    loop {
        let text = String::from_utf8_lossy(&out).into_owned();
        if text.contains(needle) {
            return text;
        }
        match tokio::time::timeout_at(deadline, ch.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => out.extend_from_slice(&data),
            Ok(Some(_)) => {}
            Ok(None) => panic!("channel closed before {needle:?}; got {text:?}"),
            Err(_) => panic!("timed out waiting for {needle:?}; got {text:?}"),
        }
    }
}

pub async fn read_to_close(ch: &mut Channel<client::Msg>) -> String {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, ch.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => out.extend_from_slice(&data),
            Ok(Some(ChannelMsg::Close)) | Ok(None) => {
                return String::from_utf8_lossy(&out).into_owned();
            }
            Ok(Some(_)) => {}
            Err(_) => panic!(
                "channel never closed; got {:?}",
                String::from_utf8_lossy(&out)
            ),
        }
    }
}

/// Until `cond` holds: the kit's one wait loop under its one step limit.
pub async fn wait_for(cond: impl Fn() -> bool) {
    wait_until(STEP_TIMEOUT, "the rig's condition", async || cond()).await;
}
