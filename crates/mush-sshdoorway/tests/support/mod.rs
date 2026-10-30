//! The end-to-end rig: a doorway on loopback TCP, an instance-side trusted
//! ingress reached in process, test hooks behind the ingress, and client
//! helpers.

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
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use russh::{Channel, ChannelMsg, client};

// ---- the instance side ----------------------------------------------------

#[derive(Default)]
pub struct State {
    pub opened: Vec<IpAddr>,
    pub failures: Vec<IpAddr>,
    pub connected: Vec<(String, u64)>,
    pub lines: Vec<String>,
    pub widths: Vec<u16>,
    pub conns: Vec<NewConn>,
}

#[derive(Default)]
pub struct Hooks {
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
    fn disconnected(&self, _: u64) -> BoxFuture<'_, ()> {
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
        let dir = std::env::temp_dir().join(format!(
            "sshdoorway-e2e-{}-{}",
            std::process::id(),
            getrandom_u64()
        ));
        std::fs::create_dir_all(&dir).unwrap();
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
        tokio::spawn(async move { d.serve(listener).await });
        Rig {
            door,
            hooks,
            connector,
            addr,
            banners: Arc::default(),
            dir,
        }
    }

    pub async fn try_client(&self) -> Result<client::Handle<Cli>, String> {
        let cfg = Arc::new(client::Config::default());
        let cli = Cli {
            banners: Arc::clone(&self.banners),
        };
        match tokio::time::timeout(Duration::from_secs(5), client::connect(cfg, self.addr, cli))
            .await
        {
            Ok(Ok(h)) => Ok(h),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("timed out".into()),
        }
    }

    pub async fn client(&self) -> client::Handle<Cli> {
        self.try_client().await.unwrap()
    }

    pub fn banners(&self) -> Vec<String> {
        self.banners.lock().unwrap().clone()
    }
}

fn getrandom_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ (std::process::id() as u64) << 32
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
        match tokio::time::timeout(Duration::from_secs(10), ch.wait())
            .await
            .expect("a reply within 10 s")
        {
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
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

pub async fn wait_for(cond: impl Fn() -> bool) {
    wait_for_secs(10, cond).await;
}

pub async fn wait_for_secs(secs: u64, cond: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while !cond() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not met within {secs} s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
