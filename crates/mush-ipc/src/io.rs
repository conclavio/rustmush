//! The async socket transport for the gateway<->server boundary (milestone
//! M16b).
//!
//! M16a proved the [`codec`](crate::codec) with in-memory round trips; this
//! module carries the same length-prefixed frames over a real tokio byte
//! stream so the two halves can run in separate processes. It is deliberately
//! transport-agnostic: an [`IpcEndpoint`] is either a `127.0.0.1:port` TCP
//! socket (the Windows-and-everywhere path) or, on Unix, a filesystem
//! `tokio::net::UnixListener` path (the Linux prod default).
//! Both accept/connect down to a boxed [`AsyncRead`]/[`AsyncWrite`] pair, so
//! the framing code above them never names a concrete stream type.
//!
//! - [`IpcEndpoint::parse`] turns a `[ipc].listen` config string into an
//!   endpoint (a parseable socket address is TCP; anything else is a Unix
//!   path where that is available).
//! - [`listen`]/[`IpcListener::accept`] are the server side; [`connect`] is
//!   the gateway side.
//! - [`FrameReader`] decodes whole [`ToServer`](crate::ToServer) /
//!   [`ToGateway`](crate::ToGateway) frames off a stream, returning `Ok(None)`
//!   on a clean EOF (the peer closed the boundary) and an error only on a
//!   truncated frame or a decode fault.
//! - [`write_frame`] serializes one message and flushes it.

use std::io;
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

use crate::codec::{CodecError, FrameDecoder, encode_frame};

/// A boxed owning read half of an IPC stream (a `TcpStream` or `UnixStream`
/// read half). Boxed so the framing layer is one type regardless of transport.
pub type BoxRead = Box<dyn AsyncRead + Unpin + Send>;
/// A boxed owning write half of an IPC stream.
pub type BoxWrite = Box<dyn AsyncWrite + Unpin + Send>;

/// Where the gateway<->server socket lives: a localhost TCP port or a Unix
/// domain socket path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcEndpoint {
    /// A TCP socket address (`127.0.0.1:port`). The only option on Windows,
    /// and the portable default everywhere.
    Tcp(SocketAddr),
    /// A Unix domain socket path (Linux/macOS prod).
    #[cfg(unix)]
    Unix(PathBuf),
}

impl IpcEndpoint {
    /// Parse a `[ipc].listen` value. A string that parses as a socket address
    /// (`127.0.0.1:6791`) is [`Tcp`](IpcEndpoint::Tcp); on Unix, anything else
    /// is taken as a filesystem path for a `Unix` socket.
    /// On non-Unix a non-address string is an error (no Unix sockets there).
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the string is neither a socket
    /// address nor (where supported) a usable Unix path.
    pub fn parse(s: &str) -> Result<IpcEndpoint, String> {
        if let Ok(addr) = s.parse::<SocketAddr>() {
            return Ok(IpcEndpoint::Tcp(addr));
        }
        #[cfg(unix)]
        {
            if !s.is_empty() {
                return Ok(IpcEndpoint::Unix(PathBuf::from(s)));
            }
        }
        Err(format!(
            "invalid [ipc].listen {s:?}: expected a socket address like \
             127.0.0.1:6791"
        ))
    }

    /// A short human label for logs.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            IpcEndpoint::Tcp(a) => a.to_string(),
            #[cfg(unix)]
            IpcEndpoint::Unix(p) => p.display().to_string(),
        }
    }
}

/// The server-side IPC listener. Wraps whichever concrete listener the
/// endpoint selected; [`accept`](IpcListener::accept) yields a boxed
/// read/write pair regardless.
pub enum IpcListener {
    /// A bound TCP listener.
    Tcp(TcpListener),
    /// A bound Unix-domain listener.
    #[cfg(unix)]
    Unix(UnixListener),
}

impl IpcListener {
    /// Accept one gateway connection, returning its boxed read/write halves.
    ///
    /// # Errors
    ///
    /// Propagates the underlying accept error.
    pub async fn accept(&self) -> io::Result<(BoxRead, BoxWrite)> {
        match self {
            IpcListener::Tcp(l) => {
                let (stream, _peer) = l.accept().await?;
                stream.set_nodelay(true).ok();
                let (r, w) = stream.into_split();
                Ok((Box::new(r), Box::new(w)))
            }
            #[cfg(unix)]
            IpcListener::Unix(l) => {
                let (stream, _peer) = l.accept().await?;
                let (r, w) = stream.into_split();
                Ok((Box::new(r), Box::new(w)))
            }
        }
    }

    /// The address/path actually bound (a TCP listener on port 0 reports the
    /// OS-chosen port here — the test harness reads it back).
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            IpcListener::Tcp(l) => l
                .local_addr()
                .map_or_else(|_| "?".to_string(), |a| a.to_string()),
            #[cfg(unix)]
            IpcListener::Unix(l) => l
                .local_addr()
                .ok()
                .and_then(|a| a.as_pathname().map(|p| p.display().to_string()))
                .unwrap_or_else(|| "?".to_string()),
        }
    }

    /// The bound TCP port, when this is a TCP listener bound to `:0` and the
    /// caller needs the OS-assigned port (used by the in-process two-process
    /// test to hand the gateway the server's address). `None` for Unix.
    #[must_use]
    pub fn tcp_addr(&self) -> Option<SocketAddr> {
        match self {
            IpcListener::Tcp(l) => l.local_addr().ok(),
            #[cfg(unix)]
            IpcListener::Unix(_) => None,
        }
    }
}

/// Bind the server-side IPC listener at `endpoint`.
///
/// For a Unix endpoint, an existing socket is probed before stale cleanup. A
/// listener that accepts the probe is left untouched and binding fails with
/// [`io::ErrorKind::AddrInUse`]; only a socket inode whose connect is refused
/// is removed. Non-socket filesystem entries and inconclusive probes are also
/// preserved and rejected. This prevents a replacement process from unlinking
/// a still-live server's pathname and binding alongside it.
///
/// # Errors
///
/// Propagates the bind error.
pub async fn listen(endpoint: &IpcEndpoint) -> io::Result<IpcListener> {
    match endpoint {
        IpcEndpoint::Tcp(addr) => Ok(IpcListener::Tcp(TcpListener::bind(addr).await?)),
        #[cfg(unix)]
        IpcEndpoint::Unix(path) => {
            prepare_unix_socket_path(path).await?;
            Ok(IpcListener::Unix(UnixListener::bind(path)?))
        }
    }
}

/// A liveness probe is deliberately short. Timeout is inconclusive and must
/// fail closed: unlinking an endpoint merely because its accept backlog is slow
/// could create two live listeners (and therefore two authoritative servers).
#[cfg(unix)]
const UNIX_SOCKET_PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// Make `path` bindable without ever unlinking an apparently live listener.
#[cfg(unix)]
async fn prepare_unix_socket_path(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};

    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("refusing to replace non-socket IPC path {}", path.display()),
        ));
    }

    match tokio::time::timeout(UNIX_SOCKET_PROBE_TIMEOUT, UnixStream::connect(path)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("IPC listener at {} is already live", path.display()),
            ));
        }
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        // It disappeared between metadata and connect. Do not run an unlink:
        // bind directly, which safely loses with AddrInUse if another owner
        // creates a replacement first.
        Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Ok(Err(error)) => {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "cannot establish that existing IPC socket {} is stale: {error}",
                    path.display()
                ),
            ));
        }
        Err(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!(
                    "timed out probing existing IPC socket {}; refusing to unlink it",
                    path.display()
                ),
            ));
        }
    }

    // Refuse to unlink if the pathname changed under the liveness probe. This
    // narrows the unavoidable pathname TOCTOU and, in particular, preserves a
    // fresh listener that replaced the refused stale inode.
    let current = match tokio::fs::symlink_metadata(path).await {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!(
                "IPC socket {} changed during stale probe; refusing to unlink it",
                path.display()
            ),
        ));
    }

    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        // The stale inode may have been cleaned by its exiting owner after our
        // refused connect. Binding the now-absent path is safe.
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Connect the gateway side to the server's IPC listener.
///
/// # Errors
///
/// Propagates the connect error (e.g. the server has not bound yet — the
/// gateway supervisor retries).
pub async fn connect(endpoint: &IpcEndpoint) -> io::Result<(BoxRead, BoxWrite)> {
    match endpoint {
        IpcEndpoint::Tcp(addr) => {
            let stream = TcpStream::connect(addr).await?;
            stream.set_nodelay(true).ok();
            let (r, w) = stream.into_split();
            Ok((Box::new(r), Box::new(w)))
        }
        #[cfg(unix)]
        IpcEndpoint::Unix(path) => {
            let stream = UnixStream::connect(path).await?;
            let (r, w) = stream.into_split();
            Ok((Box::new(r), Box::new(w)))
        }
    }
}

/// Map a codec (framing/serde) fault to an I/O error so the transport surface
/// is uniformly `io::Result`.
fn codec_io(e: CodecError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// A streaming frame reader over any [`AsyncRead`].
///
/// Feeds raw bytes into a [`FrameDecoder`] and yields one decoded message at a
/// time. A clean EOF between frames is reported as `Ok(None)`; an EOF in the
/// middle of a frame is an [`io::ErrorKind::UnexpectedEof`].
pub struct FrameReader<R> {
    inner: R,
    decoder: FrameDecoder,
    scratch: Vec<u8>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    /// Wrap a reader.
    pub fn new(inner: R) -> FrameReader<R> {
        FrameReader {
            inner,
            decoder: FrameDecoder::new(),
            scratch: vec![0u8; 16 * 1024],
        }
    }

    /// Read the next whole message, or `Ok(None)` on a clean peer disconnect.
    ///
    /// # Errors
    ///
    /// - [`io::ErrorKind::UnexpectedEof`] if the stream ended mid-frame.
    /// - [`io::ErrorKind::InvalidData`] on a decode/oversize fault.
    /// - the underlying read error otherwise.
    pub async fn read<T: DeserializeOwned>(&mut self) -> io::Result<Option<T>> {
        loop {
            if let Some(msg) = self.decoder.next_message::<T>().map_err(codec_io)? {
                return Ok(Some(msg));
            }
            let n = self.inner.read(&mut self.scratch).await?;
            if n == 0 {
                if self.decoder.pending_len() == 0 {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "ipc stream ended mid-frame",
                ));
            }
            self.decoder.feed(&self.scratch[..n]);
        }
    }
}

/// Serialize `msg` into one length-prefixed frame and flush it to `w`.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidData`] if the body is unserializable/oversize, or
/// the underlying write error.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = encode_frame(msg).map_err(codec_io)?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IPC_MIN_PROTO, IPC_VERSION, ToGateway, ToServer, WireInbound};
    use mush_core::ObjId;

    #[cfg(unix)]
    fn unique_unix_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "rustmush-ipc-{tag}-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[tokio::test]
    async fn tcp_round_trips_both_directions() {
        let listener = listen(&IpcEndpoint::parse("127.0.0.1:0").unwrap())
            .await
            .unwrap();
        let ep = IpcEndpoint::Tcp(listener.tcp_addr().unwrap());

        let server = tokio::spawn(async move {
            let (r, mut w) = listener.accept().await.unwrap();
            // Server greets, then echoes one inbound back as a Deliver marker.
            write_frame(
                &mut w,
                &ToGateway::Hello {
                    proto: IPC_VERSION,
                    min_proto: IPC_MIN_PROTO,
                    server: "test".to_string(),
                    features: vec![],
                    panels: vec![],
                },
            )
            .await
            .unwrap();
            let mut reader = FrameReader::new(r);
            let msg: ToServer = reader.read().await.unwrap().unwrap();
            assert!(matches!(msg, ToServer::Inbound(WireInbound::Cmd { .. })));
            // Clean shutdown of the boundary.
            write_frame(&mut w, &ToGateway::Bye).await.unwrap();
            // Next read is a clean EOF.
            assert!(reader.read::<ToServer>().await.unwrap().is_none());
        });

        let (r, mut w) = connect(&ep).await.unwrap();
        let mut reader = FrameReader::new(r);
        let hello: ToGateway = reader.read().await.unwrap().unwrap();
        match hello {
            ToGateway::Hello { proto, .. } => assert_eq!(proto, IPC_VERSION),
            other => panic!("expected Hello, got {other:?}"),
        }
        write_frame(
            &mut w,
            &ToServer::Inbound(WireInbound::Cmd {
                conn: 1,
                actor: ObjId(1),
                line: "look".to_string(),
            }),
        )
        .await
        .unwrap();
        let bye: ToGateway = reader.read().await.unwrap().unwrap();
        assert!(matches!(bye, ToGateway::Bye));
        drop(w);
        server.await.unwrap();
    }

    /// A hot-restart candidate must never steal a live Unix pathname. Unix
    /// permits unlinking a bound socket while its listener remains alive, so
    /// the old unconditional cleanup would let both servers coexist.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_listen_refuses_to_unlink_a_live_listener() {
        let path = unique_unix_path("live");
        let endpoint = IpcEndpoint::Unix(path.clone());
        let first = listen(&endpoint).await.expect("first listener binds");

        let error = match listen(&endpoint).await {
            Ok(_) => panic!("a live listener must not be replaced"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);

        // Its pathname is still usable after the refused replacement.
        let stream = UnixStream::connect(&path)
            .await
            .expect("original listener pathname remains live");
        drop(stream);
        drop(first);
        let _ = std::fs::remove_file(path);
    }

    /// A crashed Unix listener leaves an inode that refuses connections. That
    /// exact stale case remains recoverable without weakening the live-listener
    /// guard above.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_listen_replaces_a_refused_stale_socket() {
        let path = unique_unix_path("stale");
        let endpoint = IpcEndpoint::Unix(path.clone());
        let stale = UnixListener::bind(&path).expect("stale listener binds");
        drop(stale);

        let replacement = listen(&endpoint)
            .await
            .expect("refused stale socket is replaced");
        let stream = UnixStream::connect(&path)
            .await
            .expect("replacement listener is reachable");
        drop(stream);
        drop(replacement);
        let _ = std::fs::remove_file(path);
    }

    /// Misconfiguration must not turn IPC startup into an arbitrary unlink of
    /// an ordinary file (or symlink) at the configured path.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_listen_preserves_non_socket_paths() {
        let path = unique_unix_path("ordinary");
        std::fs::write(&path, b"keep me").expect("create ordinary path");
        let endpoint = IpcEndpoint::Unix(path.clone());

        let error = match listen(&endpoint).await {
            Ok(_) => panic!("ordinary path must not be unlinked"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
        let _ = std::fs::remove_file(path);
    }
}
