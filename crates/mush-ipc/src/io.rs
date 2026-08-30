//! The async socket transport for the gateway<->server boundary (milestone
//! M16b).
//!
//! M16a proved the [`codec`](crate::codec) with in-memory round trips; this
//! module carries the same length-prefixed frames over a real tokio byte
//! stream so the two halves can run in separate processes. It is deliberately
//! transport-agnostic: an [`IpcEndpoint`] is either a `127.0.0.1:port` TCP
//! socket (the Windows-and-everywhere path) or, on Unix, a filesystem
//! [`UnixListener`](tokio::net::UnixListener) path (the Linux prod default).
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
use std::path::PathBuf;

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
    /// is taken as a filesystem path for a [`Unix`](IpcEndpoint::Unix) socket.
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
/// For a Unix endpoint a stale socket file left by a previous run is removed
/// first (best effort) so the bind does not fail with `EADDRINUSE`.
///
/// # Errors
///
/// Propagates the bind error.
pub async fn listen(endpoint: &IpcEndpoint) -> io::Result<IpcListener> {
    match endpoint {
        IpcEndpoint::Tcp(addr) => Ok(IpcListener::Tcp(TcpListener::bind(addr).await?)),
        #[cfg(unix)]
        IpcEndpoint::Unix(path) => {
            // A leftover socket file from a crashed run blocks re-bind.
            let _ = tokio::fs::remove_file(path).await;
            Ok(IpcListener::Unix(UnixListener::bind(path)?))
        }
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
    use crate::{IPC_VERSION, ToGateway, ToServer, WireInbound};
    use mush_core::ObjId;

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
                    server: "test".to_string(),
                    features: vec![],
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
}
