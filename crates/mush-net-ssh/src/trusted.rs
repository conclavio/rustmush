//! The **trusted ingress** (doc 29 §4): a second SSH listener, a Unix socket
//! only sshdoorway can reach, whose connections begin with the
//! `mush-sshdoorway-proto` preamble.
//!
//! The doorway terminates the player's SSH session, so on this socket the SSH
//! peer is always the doorway. The preamble restores what that hides:
//!
//! - the player's **real address**, which becomes [`ConnHandler`]'s `addr`, so
//!   every per-IP gate (the connection cap, `is_ip_blocked`, the failure
//!   tempban, the welcome screen's re-checks) keys on the player, never on the
//!   doorway;
//! - a key the doorway **verified** the player holds (russh checked the
//!   signature downstream). The instance still makes the authorization
//!   decision through [`SshHooks::auth_account_key`]; an accepted key binds
//!   exactly one username, and `auth_none` for that username is the login.
//!
//! Passwords never cross the preamble: the doorway relays them inside the
//! upstream SSH authentication, so the password path and its failure counting
//! run here unchanged.
//!
//! The public TCP listener never reads a preamble — the trust is the socket's
//! permissions (mode 0660 in a setgid directory owned by group `sshdoorway`,
//! doc 29a A-1).
//!
//! [`TrustedIngress::serve`] is generic over the stream, so the preamble runs
//! under test on every platform (`tokio::io::duplex`);
//! [`SshServer::bind_trusted`](crate::SshServer::bind_trusted) is the Unix
//! wrapper.

use std::sync::Arc;
use std::time::Duration;

use mush_sshdoorway_proto::{
    self as proto, DoorwayMsg, IngressMsg, MAX_BEGINS, MAX_KEY_LEN, MAX_KEY_QUERIES, ProtoError,
};
use russh::server::Config;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::SshError;
use crate::hooks::{AuthResult, SshHooks};
use crate::server::{
    Attested, BLOCKED_FALLBACK, ConnHandler, Ingress, block_notice, is_front_door, server_config,
    split_account_user,
};

/// The whole preamble — from the first byte to `Ready` — must finish within
/// this. A doorway that stalls is holding a per-IP slot it should not.
pub const PREAMBLE_TIMEOUT: Duration = Duration::from_secs(30);

/// One instance's trusted ingress: the shared russh configuration plus the
/// connection machinery the public port uses. Cheap to clone.
#[derive(Clone)]
pub struct TrustedIngress {
    config: Arc<Config>,
    ingress: Ingress,
}

impl std::fmt::Debug for TrustedIngress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedIngress").finish_non_exhaustive()
    }
}

impl TrustedIngress {
    /// Build the ingress. The host key at `host_key_path` is the same file the
    /// public port uses (loaded, or generated on first boot).
    pub fn new(
        host_key_path: &std::path::Path,
        banner: &str,
        hooks: Arc<dyn SshHooks>,
    ) -> Result<TrustedIngress, SshError> {
        Ok(TrustedIngress {
            config: server_config(host_key_path)?,
            ingress: Ingress::new(hooks, banner),
        })
    }

    /// Serve one doorway connection: the preamble, then SSH on the same
    /// stream until the session ends. Never panics on hostile input; every
    /// failure closes the stream and releases any slot the `Hello` claimed.
    pub async fn serve<S>(&self, mut stream: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let handler = match tokio::time::timeout(
            PREAMBLE_TIMEOUT,
            preamble(&self.ingress, &mut stream),
        )
        .await
        {
            Ok(Ok(Some(handler))) => handler,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                tracing::debug!(%error, "trusted ingress: preamble failed");
                return;
            }
            Err(_) => {
                tracing::info!("trusted ingress: preamble timed out");
                return;
            }
        };
        match russh::server::run_stream(Arc::clone(&self.config), stream, handler).await {
            Ok(session) => {
                if let Err(error) = session.await {
                    tracing::debug!(%error, "trusted ingress: SSH session ended with error");
                }
            }
            Err(error) => tracing::debug!(%error, "trusted ingress: SSH start failed"),
        }
    }
}

/// Run the instance side of the preamble. `Ok(Some(handler))` means `Ready`
/// was sent and SSH follows; `Ok(None)` is an answered refusal (`Pong`,
/// `Refused`, `Blocked`, a version mismatch).
async fn preamble<S>(ingress: &Ingress, stream: &mut S) -> Result<Option<ConnHandler>, ProtoError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hooks = Arc::clone(&ingress.hooks);
    let peer = match proto::read_msg::<_, DoorwayMsg>(stream).await {
        Ok(DoorwayMsg::Ping) => {
            proto::write_msg(stream, &IngressMsg::Pong).await?;
            return Ok(None);
        }
        Ok(DoorwayMsg::Hello { peer, conn, .. }) => {
            tracing::debug!(doorway_conn = conn, %peer, "trusted ingress: hello");
            peer
        }
        Ok(other) => {
            tracing::info!(?other, "trusted ingress: first frame was not Hello");
            return Ok(None);
        }
        Err(ProtoError::Version(v)) => {
            tracing::warn!(
                version = v,
                "trusted ingress: doorway speaks another preamble version"
            );
            proto::write_msg(
                stream,
                &IngressMsg::VersionMismatch {
                    supported: proto::VERSION,
                },
            )
            .await?;
            return Ok(None);
        }
        Err(e) => return Err(e),
    };

    let addr = peer.ip();
    let counted = hooks.connection_opened(addr);
    // Built now so its `Drop` releases the slot on every path below.
    let mut handler = ingress.handler_for(addr, counted);
    if !counted {
        proto::write_msg(stream, &IngressMsg::Refused).await?;
        return Ok(None);
    }
    if hooks.is_ip_blocked(addr) {
        let notice = hooks
            .block_reason(addr)
            .map_or_else(|| BLOCKED_FALLBACK.to_owned(), block_notice);
        proto::write_msg(stream, &IngressMsg::Blocked { notice }).await?;
        return Ok(None);
    }
    proto::write_msg(stream, &IngressMsg::Open).await?;

    let mut queries = 0usize;
    let mut begins = 0usize;
    loop {
        match proto::read_msg::<_, DoorwayMsg>(stream).await? {
            DoorwayMsg::KeyQuery { user, key } => {
                queries += 1;
                if queries > MAX_KEY_QUERIES {
                    tracing::info!(%addr, "trusted ingress: too many key queries");
                    return Ok(None);
                }
                let acceptable = match key_account(&user, &key) {
                    Some(account) => matches!(
                        hooks.auth_account_key(account, &key).await,
                        AuthResult::Ok { .. }
                    ),
                    None => false,
                };
                proto::write_msg(stream, &IngressMsg::KeyReply { acceptable }).await?;
            }
            DoorwayMsg::Begin { user, attested_key } => {
                begins += 1;
                if begins > MAX_BEGINS || !proto::user_is_forwardable(&user) {
                    tracing::info!(%addr, "trusted ingress: refused Begin");
                    return Ok(None);
                }
                let Some(key) = attested_key else {
                    proto::write_msg(stream, &IngressMsg::Ready).await?;
                    return Ok(Some(handler));
                };
                let Some(account) = key_account(&user, &key) else {
                    proto::write_msg(stream, &IngressMsg::BadCreds).await?;
                    continue;
                };
                match hooks.auth_account_key(account, &key).await {
                    AuthResult::Ok {
                        account,
                        characters,
                    } => {
                        handler.attested = Some(Attested {
                            user,
                            account,
                            characters,
                        });
                        proto::write_msg(stream, &IngressMsg::Ready).await?;
                        return Ok(Some(handler));
                    }
                    // Not counted as a failure, exactly like a key miss on the
                    // public port's signed-key callback: the player proved they
                    // hold the key, it is just not registered here.
                    AuthResult::BadCreds => {
                        proto::write_msg(stream, &IngressMsg::BadCreds).await?;
                    }
                    AuthResult::Banned => {
                        proto::write_msg(stream, &IngressMsg::Banned).await?;
                    }
                }
            }
            other => {
                tracing::info!(?other, %addr, "trusted ingress: unexpected preamble frame");
                return Ok(None);
            }
        }
    }
}

/// The account a key may log into for `user`, or `None` when no key login is
/// possible (a front-door name, an unforwardable name, an oversized key).
fn key_account<'a>(user: &'a str, key: &str) -> Option<&'a str> {
    if !proto::user_is_forwardable(user) || key.is_empty() || key.len() > MAX_KEY_LEN {
        return None;
    }
    let (account, _) = split_account_user(user);
    if account.is_empty() || is_front_door(account) {
        return None;
    }
    Some(account)
}

/// Configuration for [`SshServer::bind_trusted`](crate::SshServer::bind_trusted).
#[derive(Clone, Debug)]
pub struct TrustedConfig {
    /// The socket path, e.g. `/run/rustmush-ssh/dev1.sock`. Its directory must
    /// exist (tmpfiles creates it setgid, group `sshdoorway`).
    pub socket_path: std::path::PathBuf,
    /// The instance's host key (the same file the public port uses).
    pub host_key_path: std::path::PathBuf,
    /// Welcome-screen banner (as [`SshConfig::banner`](crate::SshConfig)).
    pub banner: String,
}

/// A bound trusted ingress, not yet accepting.
#[cfg(unix)]
pub struct BoundTrustedServer {
    path: std::path::PathBuf,
    listener: tokio::net::UnixListener,
    ingress: TrustedIngress,
}

#[cfg(unix)]
impl std::fmt::Debug for BoundTrustedServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundTrustedServer")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
impl BoundTrustedServer {
    /// Bind `cfg.socket_path`: a stale **socket** at the path is replaced, any
    /// other file is an error (never deleted), and the new socket is made
    /// mode 0660 so only the directory's group (`sshdoorway`) can connect.
    pub async fn bind(
        cfg: TrustedConfig,
        hooks: Arc<dyn SshHooks>,
    ) -> Result<BoundTrustedServer, SshError> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};

        let ingress = TrustedIngress::new(&cfg.host_key_path, &cfg.banner, hooks)?;
        let bind_err = |source: std::io::Error| SshError::BindTrusted {
            path: cfg.socket_path.display().to_string(),
            source,
        };
        match std::fs::symlink_metadata(&cfg.socket_path) {
            Ok(meta) if meta.file_type().is_socket() => {
                std::fs::remove_file(&cfg.socket_path).map_err(bind_err)?;
            }
            Ok(_) => {
                return Err(bind_err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "the path exists and is not a socket; refusing to replace it",
                )));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(bind_err(e)),
        }
        let listener = tokio::net::UnixListener::bind(&cfg.socket_path).map_err(bind_err)?;
        std::fs::set_permissions(&cfg.socket_path, std::fs::Permissions::from_mode(0o660))
            .map_err(bind_err)?;
        tracing::info!(path = %cfg.socket_path.display(), "SSH trusted ingress listening");
        Ok(BoundTrustedServer {
            path: cfg.socket_path,
            listener,
            ingress,
        })
    }

    /// The socket path.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Accept forever, one task per doorway connection.
    pub async fn run(self) -> Result<(), SshError> {
        loop {
            let (stream, _) = self.listener.accept().await.map_err(SshError::Server)?;
            let ingress = self.ingress.clone();
            tokio::spawn(async move { ingress.serve(stream).await });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, SocketAddr};
    use std::sync::Mutex;

    use futures::future::BoxFuture;
    use tokio::io::AsyncReadExt;

    use crate::hooks::{BlockReason, CharSummary, CreateErr, NewConn};

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGZha2Uta2V5LWZvci10ZXN0cy0wMDAwMDAwMDAw";

    #[derive(Default)]
    struct Hooks {
        opened: Mutex<Vec<IpAddr>>,
        open_now: Mutex<i64>,
        cap_full: bool,
        blocked: bool,
        key_calls: Mutex<Vec<(String, String)>>,
    }

    impl SshHooks for Hooks {
        fn auth_account<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, AuthResult> {
            Box::pin(async { AuthResult::BadCreds })
        }
        fn auth_account_key<'a>(&'a self, a: &'a str, k: &'a str) -> BoxFuture<'a, AuthResult> {
            self.key_calls
                .lock()
                .unwrap()
                .push((a.to_owned(), k.to_owned()));
            Box::pin(async move {
                match (a, k) {
                    ("faenix", KEY) => AuthResult::Ok {
                        account: "faenix".into(),
                        characters: vec![CharSummary {
                            key: 1,
                            name: "Kira".into(),
                        }],
                    },
                    ("banned", KEY) => AuthResult::Banned,
                    _ => AuthResult::BadCreds,
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
            _: NewConn,
            _: &'a str,
            _: u64,
        ) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async { Err("no".to_owned()) })
        }
        fn line(&self, _: u64, _: String) -> BoxFuture<'_, ()> {
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
        fn auth_failed(&self, _: IpAddr) {}
        fn connection_opened(&self, addr: IpAddr) -> bool {
            self.opened.lock().unwrap().push(addr);
            if self.cap_full {
                return false;
            }
            *self.open_now.lock().unwrap() += 1;
            true
        }
        fn connection_closed(&self, _: IpAddr) {
            *self.open_now.lock().unwrap() -= 1;
        }
    }

    fn ingress(hooks: Arc<Hooks>) -> Ingress {
        Ingress::new(hooks, "banner")
    }

    fn hello() -> DoorwayMsg {
        DoorwayMsg::Hello {
            peer: "198.51.100.7:40000".parse::<SocketAddr>().unwrap(),
            local: "203.0.113.1:22".parse::<SocketAddr>().unwrap(),
            conn: 1,
        }
    }

    async fn send(s: &mut tokio::io::DuplexStream, m: DoorwayMsg) {
        proto::write_msg(s, &m).await.unwrap();
    }

    async fn recv(s: &mut tokio::io::DuplexStream) -> IngressMsg {
        proto::read_msg(s).await.unwrap()
    }

    /// Runs the preamble on one end; the test drives the other. Returns the
    /// handler (if `Ready`) so the test can inspect it before dropping it.
    async fn run(
        hooks: Arc<Hooks>,
        script: impl AsyncFnOnce(&mut tokio::io::DuplexStream),
    ) -> Option<ConnHandler> {
        let (mut doorway, mut instance) = tokio::io::duplex(64 * 1024);
        let ing = ingress(hooks);
        let server = async move { preamble(&ing, &mut instance).await.ok().flatten() };
        let client = async move {
            script(&mut doorway).await;
            drop(doorway);
        };
        let (handler, ()) = tokio::join!(server, client);
        handler
    }

    #[tokio::test]
    async fn ping_is_answered_and_claims_nothing() {
        let hooks = Arc::new(Hooks::default());
        let h = run(Arc::clone(&hooks), async |s| {
            send(s, DoorwayMsg::Ping).await;
            assert_eq!(recv(s).await, IngressMsg::Pong);
        })
        .await;
        assert!(h.is_none());
        assert!(hooks.opened.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hello_claims_the_players_address_and_releases_it_exactly_once() {
        let hooks = Arc::new(Hooks::default());
        let h = run(Arc::clone(&hooks), async |s| {
            send(s, hello()).await;
            assert_eq!(recv(s).await, IngressMsg::Open);
            send(
                s,
                DoorwayMsg::Begin {
                    user: "faenix".into(),
                    attested_key: None,
                },
            )
            .await;
            assert_eq!(recv(s).await, IngressMsg::Ready);
        })
        .await
        .unwrap();
        assert_eq!(
            *hooks.opened.lock().unwrap(),
            vec!["198.51.100.7".parse::<IpAddr>().unwrap()]
        );
        assert_eq!(h.addr, "198.51.100.7".parse::<IpAddr>().unwrap());
        assert!(h.attested.is_none());
        assert_eq!(*hooks.open_now.lock().unwrap(), 1);
        drop(h);
        assert_eq!(*hooks.open_now.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_preamble_abandoned_after_hello_releases_the_slot() {
        let hooks = Arc::new(Hooks::default());
        let h = run(Arc::clone(&hooks), async |s| {
            send(s, hello()).await;
            assert_eq!(recv(s).await, IngressMsg::Open);
            // Garbage, then close.
            s.write_all(&[0, 0, 0, 3, 0xc1, 0xc1, 0xc1]).await.unwrap();
        })
        .await;
        assert!(h.is_none());
        assert_eq!(*hooks.open_now.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn over_the_cap_is_refused_and_releases_nothing() {
        let hooks = Arc::new(Hooks {
            cap_full: true,
            ..Hooks::default()
        });
        let h = run(Arc::clone(&hooks), async |s| {
            send(s, hello()).await;
            assert_eq!(recv(s).await, IngressMsg::Refused);
        })
        .await;
        assert!(h.is_none());
        assert_eq!(*hooks.open_now.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_blocked_address_is_told_why() {
        let hooks = Arc::new(Hooks {
            blocked: true,
            ..Hooks::default()
        });
        run(Arc::clone(&hooks), async |s| {
            send(s, hello()).await;
            assert_eq!(
                recv(s).await,
                IngressMsg::Blocked {
                    notice: "Connections from your address are not permitted.".into()
                }
            );
        })
        .await;
        assert_eq!(*hooks.open_now.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn key_queries_answer_without_binding_and_are_bounded() {
        let hooks = Arc::new(Hooks::default());
        let h = run(Arc::clone(&hooks), async |s| {
            send(s, hello()).await;
            recv(s).await;
            for (user, want) in [
                ("faenix", true),
                ("faenix+Kira", true),
                ("other", false),
                ("play", false),
                ("", false),
            ] {
                send(
                    s,
                    DoorwayMsg::KeyQuery {
                        user: user.into(),
                        key: KEY.into(),
                    },
                )
                .await;
                assert_eq!(recv(s).await, IngressMsg::KeyReply { acceptable: want });
            }
            for _ in 5..MAX_KEY_QUERIES {
                send(
                    s,
                    DoorwayMsg::KeyQuery {
                        user: "x".into(),
                        key: KEY.into(),
                    },
                )
                .await;
                recv(s).await;
            }
            // One past the limit: the instance closes the stream.
            send(
                s,
                DoorwayMsg::KeyQuery {
                    user: "x".into(),
                    key: KEY.into(),
                },
            )
            .await;
            let mut rest = Vec::new();
            s.read_to_end(&mut rest).await.unwrap();
            assert!(rest.is_empty());
        })
        .await;
        assert!(h.is_none());
        assert_eq!(*hooks.open_now.lock().unwrap(), 0);
        // The front door and the empty name never reached the account store.
        let calls = hooks.key_calls.lock().unwrap();
        assert!(calls.iter().all(|(a, _)| a != "play" && !a.is_empty()));
    }

    #[tokio::test]
    async fn an_attested_begin_binds_exactly_that_user() {
        let hooks = Arc::new(Hooks::default());
        let h = run(Arc::clone(&hooks), async |s| {
            send(s, hello()).await;
            recv(s).await;
            send(
                s,
                DoorwayMsg::Begin {
                    user: "other".into(),
                    attested_key: Some(KEY.into()),
                },
            )
            .await;
            assert_eq!(recv(s).await, IngressMsg::BadCreds);
            send(
                s,
                DoorwayMsg::Begin {
                    user: "banned".into(),
                    attested_key: Some(KEY.into()),
                },
            )
            .await;
            assert_eq!(recv(s).await, IngressMsg::Banned);
            send(
                s,
                DoorwayMsg::Begin {
                    user: "faenix+Kira".into(),
                    attested_key: Some(KEY.into()),
                },
            )
            .await;
            assert_eq!(recv(s).await, IngressMsg::Ready);
        })
        .await
        .unwrap();
        let att = h.attested.as_ref().unwrap();
        assert_eq!(att.user, "faenix+Kira");
        assert_eq!(att.account, "faenix");
    }

    #[tokio::test]
    async fn begins_are_bounded_and_front_door_keys_never_bind() {
        let hooks = Arc::new(Hooks::default());
        let h = run(Arc::clone(&hooks), async |s| {
            send(s, hello()).await;
            recv(s).await;
            for _ in 0..MAX_BEGINS {
                send(
                    s,
                    DoorwayMsg::Begin {
                        user: "play".into(),
                        attested_key: Some(KEY.into()),
                    },
                )
                .await;
                assert_eq!(recv(s).await, IngressMsg::BadCreds);
            }
            send(
                s,
                DoorwayMsg::Begin {
                    user: "faenix".into(),
                    attested_key: Some(KEY.into()),
                },
            )
            .await;
            let mut rest = Vec::new();
            s.read_to_end(&mut rest).await.unwrap();
            assert!(rest.is_empty(), "the fifth Begin closes the stream");
        })
        .await;
        assert!(h.is_none());
    }

    #[tokio::test]
    async fn unforwardable_users_and_out_of_order_frames_close_the_stream() {
        for script in 0..3 {
            let hooks = Arc::new(Hooks::default());
            let h = run(Arc::clone(&hooks), async |s| match script {
                0 => {
                    send(
                        s,
                        DoorwayMsg::Begin {
                            user: "x".into(),
                            attested_key: None,
                        },
                    )
                    .await;
                }
                1 => {
                    send(s, hello()).await;
                    recv(s).await;
                    send(s, hello()).await;
                }
                _ => {
                    send(s, hello()).await;
                    recv(s).await;
                    send(
                        s,
                        DoorwayMsg::Begin {
                            user: "a\r\nb".into(),
                            attested_key: None,
                        },
                    )
                    .await;
                }
            })
            .await;
            assert!(h.is_none(), "script {script}");
            assert_eq!(*hooks.open_now.lock().unwrap(), 0, "script {script}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_unix_socket_is_0660_replaces_a_stale_socket_and_never_a_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "mush-trusted-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let socket_path = dir.join("dev1.sock");
        let cfg = || TrustedConfig {
            socket_path: socket_path.clone(),
            host_key_path: dir.join("host"),
            banner: "b".into(),
        };
        let hooks: Arc<dyn SshHooks> = Arc::new(Hooks::default());
        let first = BoundTrustedServer::bind(cfg(), Arc::clone(&hooks))
            .await
            .unwrap();
        let mode = std::fs::metadata(&socket_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o660);
        drop(first);
        // The socket file is left behind (stale); a rebind replaces it.
        let second = BoundTrustedServer::bind(cfg(), Arc::clone(&hooks))
            .await
            .unwrap();
        // And it answers a Ping.
        let ingress = second.ingress.clone();
        let listener = second.listener;
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            ingress.serve(s).await;
        });
        let mut s = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
        proto::write_msg(&mut s, &DoorwayMsg::Ping).await.unwrap();
        let pong: IngressMsg = proto::read_msg(&mut s).await.unwrap();
        assert_eq!(pong, IngressMsg::Pong);

        let file = dir.join("not-a-socket");
        std::fs::write(&file, b"keep me").unwrap();
        let err = BoundTrustedServer::bind(
            TrustedConfig {
                socket_path: file.clone(),
                ..cfg()
            },
            hooks,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not a socket"), "{err}");
        assert_eq!(std::fs::read(&file).unwrap(), b"keep me");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn another_version_is_told_ours() {
        #[derive(serde::Serialize)]
        struct Future {
            version: u16,
            msg: &'static str,
        }
        let hooks = Arc::new(Hooks::default());
        run(Arc::clone(&hooks), async |s| {
            let body = rmp_serde::to_vec_named(&Future {
                version: 9,
                msg: "Hello",
            })
            .unwrap();
            s.write_all(&u32::try_from(body.len()).unwrap().to_be_bytes())
                .await
                .unwrap();
            s.write_all(&body).await.unwrap();
            assert_eq!(
                recv(s).await,
                IngressMsg::VersionMismatch {
                    supported: proto::VERSION
                }
            );
        })
        .await;
        assert!(hooks.opened.lock().unwrap().is_empty());
    }

    use tokio::io::AsyncWriteExt;
}
