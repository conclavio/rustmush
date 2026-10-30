//! The doorway → instance hop: dialing a service, the doorway side of the
//! preamble (doc 29 §4.2), and the upstream russh client.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_sshdoorway_proto::{
    self as proto, DoorwayMsg, IngressMsg, MAX_BEGINS, MAX_KEY_QUERIES, ProtoError,
};
use russh::client;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::config::ServiceCfg;

/// Every upstream step (dial, a preamble round trip, the SSH handshake, an
/// auth attempt) must finish within this (29a R-2).
pub const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// A byte stream to an instance's trusted ingress.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static> Stream for T {}

/// A boxed [`Stream`].
pub type BoxStream = Box<dyn Stream>;

/// How the doorway reaches a service. Production dials the service's Unix
/// socket (`UnixConnector`, Unix only); the tests dial an in-process ingress.
pub trait Connector: Send + Sync + 'static {
    /// Open one stream to `service`.
    fn connect<'a>(&'a self, service: &'a ServiceCfg) -> BoxFuture<'a, std::io::Result<BoxStream>>;
}

/// Dials `service.socket`.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UnixConnector;

#[cfg(unix)]
impl Connector for UnixConnector {
    fn connect<'a>(&'a self, service: &'a ServiceCfg) -> BoxFuture<'a, std::io::Result<BoxStream>> {
        Box::pin(async move {
            let s = tokio::net::UnixStream::connect(&service.socket).await?;
            Ok(Box::new(s) as BoxStream)
        })
    }
}

/// An upstream failure.
#[derive(Debug, thiserror::Error)]
pub enum UpError {
    /// Dialing failed.
    #[error("dial: {0}")]
    Dial(#[from] std::io::Error),
    /// The preamble failed.
    #[error("preamble: {0}")]
    Proto(#[from] ProtoError),
    /// The instance answered something the protocol does not allow here.
    #[error("unexpected preamble reply {0:?}")]
    Unexpected(IngressMsg),
    /// The instance speaks another preamble version.
    #[error("instance speaks preamble version {0}")]
    Version(u16),
    /// A per-stream preamble limit would be exceeded.
    #[error("preamble limit reached")]
    Limit,
    /// The upstream SSH session failed.
    #[error("upstream SSH: {0}")]
    Ssh(#[from] russh::Error),
    /// A step took longer than [`UPSTREAM_TIMEOUT`].
    #[error("upstream timed out")]
    Timeout,
}

/// Bound one upstream step by [`UPSTREAM_TIMEOUT`].
pub async fn timed<T, E>(f: impl Future<Output = Result<T, E>>) -> Result<T, UpError>
where
    UpError: From<E>,
{
    match tokio::time::timeout(UPSTREAM_TIMEOUT, f).await {
        Ok(r) => r.map_err(UpError::from),
        Err(_) => Err(UpError::Timeout),
    }
}

/// The answer to `Hello`.
pub enum HelloOutcome {
    /// The slot is claimed; the preamble continues.
    Open(Preamble),
    /// The player's address is blocked; tell them `notice`.
    Blocked(String),
    /// The player's address is over the instance's connection cap.
    Refused,
}

/// The answer to `Begin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeginOutcome {
    /// SSH follows.
    Ready,
    /// The attested key is not registered to the account.
    BadCreds,
    /// The account is banned.
    Banned,
}

/// An open preamble on one stream.
pub struct Preamble {
    stream: BoxStream,
    queries: usize,
    begins: usize,
}

impl std::fmt::Debug for Preamble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preamble")
            .field("queries", &self.queries)
            .field("begins", &self.begins)
            .finish_non_exhaustive()
    }
}

async fn reply(stream: &mut BoxStream) -> Result<IngressMsg, UpError> {
    match proto::read_msg::<_, IngressMsg>(stream).await? {
        IngressMsg::VersionMismatch { supported } => Err(UpError::Version(supported)),
        other => Ok(other),
    }
}

impl Preamble {
    /// Dial `service` and send `Hello` for the player at `peer`.
    pub async fn hello(
        connector: &dyn Connector,
        service: &ServiceCfg,
        peer: SocketAddr,
        local: SocketAddr,
        conn: u64,
    ) -> Result<HelloOutcome, UpError> {
        let mut stream = timed(connector.connect(service)).await?;
        timed(proto::write_msg(
            &mut stream,
            &DoorwayMsg::Hello { peer, local, conn },
        ))
        .await?;
        match timed(reply(&mut stream)).await? {
            IngressMsg::Open => Ok(HelloOutcome::Open(Preamble {
                stream,
                queries: 0,
                begins: 0,
            })),
            IngressMsg::Blocked { notice } => Ok(HelloOutcome::Blocked(notice)),
            IngressMsg::Refused => Ok(HelloOutcome::Refused),
            other => Err(UpError::Unexpected(other)),
        }
    }

    /// Ask whether the instance would accept `key` for `user`.
    pub async fn key_query(&mut self, user: &str, key: &str) -> Result<bool, UpError> {
        if self.queries >= MAX_KEY_QUERIES {
            return Err(UpError::Limit);
        }
        self.queries += 1;
        timed(proto::write_msg(
            &mut self.stream,
            &DoorwayMsg::KeyQuery {
                user: user.to_owned(),
                key: key.to_owned(),
            },
        ))
        .await?;
        match timed(reply(&mut self.stream)).await? {
            IngressMsg::KeyReply { acceptable } => Ok(acceptable),
            other => Err(UpError::Unexpected(other)),
        }
    }

    /// Send `Begin`. On [`BeginOutcome::Ready`] the stream carries SSH next.
    pub async fn begin(
        &mut self,
        user: &str,
        attested_key: Option<String>,
    ) -> Result<BeginOutcome, UpError> {
        if self.begins >= MAX_BEGINS {
            return Err(UpError::Limit);
        }
        self.begins += 1;
        timed(proto::write_msg(
            &mut self.stream,
            &DoorwayMsg::Begin {
                user: user.to_owned(),
                attested_key,
            },
        ))
        .await?;
        match timed(reply(&mut self.stream)).await? {
            IngressMsg::Ready => Ok(BeginOutcome::Ready),
            IngressMsg::BadCreds => Ok(BeginOutcome::BadCreds),
            IngressMsg::Banned => Ok(BeginOutcome::Banned),
            other => Err(UpError::Unexpected(other)),
        }
    }

    /// The stream, for SSH, after `Ready`.
    #[must_use]
    pub fn into_stream(self) -> BoxStream {
        self.stream
    }
}

/// The readiness probe (29a A-3): `Ping` → `Pong`, claiming nothing.
pub async fn ping(connector: &dyn Connector, service: &ServiceCfg) -> Result<(), UpError> {
    let mut stream = timed(connector.connect(service)).await?;
    timed(proto::write_msg(&mut stream, &DoorwayMsg::Ping)).await?;
    match timed(reply(&mut stream)).await? {
        IngressMsg::Pong => Ok(()),
        other => Err(UpError::Unexpected(other)),
    }
}

/// Where the upstream session's end is recorded, for the player.
pub type DisconnectSlot = Arc<Mutex<Option<String>>>;

/// The upstream russh client's handler.
pub struct UpHandler {
    service: String,
    disconnect: DisconnectSlot,
}

impl client::Handler for UpHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            russh::keys::PublicKeyOrCertificate::Certificate(cert) => {
                cert.public_key().clone().into()
            }
        };
        // The trusted hop's host key is not verified: the socket's
        // permissions are the trust (D-29-9). Logged once per connection.
        tracing::debug!(
            service = %self.service,
            fingerprint = %key.fingerprint(russh::keys::HashAlg::Sha256),
            "upstream host key"
        );
        Ok(true)
    }

    async fn disconnected(
        &mut self,
        reason: client::DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        let message = match reason {
            client::DisconnectReason::ReceivedDisconnect(info) => info.message,
            client::DisconnectReason::Error(e) => {
                tracing::debug!(service = %self.service, error = %e, "upstream SSH ended");
                String::new()
            }
        };
        if let Ok(mut slot) = self.disconnect.lock() {
            *slot = Some(message);
        }
        Ok(())
    }
}

/// The upstream client configuration.
#[must_use]
pub fn client_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..client::Config::default()
    })
}

/// Start the upstream SSH session on a preamble stream that answered `Ready`.
pub async fn start_ssh(
    config: Arc<client::Config>,
    stream: BoxStream,
    service: &str,
) -> Result<(client::Handle<UpHandler>, DisconnectSlot), UpError> {
    let disconnect: DisconnectSlot = Arc::new(Mutex::new(None));
    let handler = UpHandler {
        service: service.to_owned(),
        disconnect: Arc::clone(&disconnect),
    };
    let handle = timed(client::connect_stream(config, stream, handler)).await?;
    Ok((handle, disconnect))
}
