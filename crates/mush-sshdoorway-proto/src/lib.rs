//! The sshdoorway preamble (doc 29 §4.2).
//!
//! sshdoorway terminates the player's SSH session and opens a second one to
//! the instance over the instance's **trusted ingress**, a Unix socket only
//! the doorway can reach. Before the first SSH byte on that socket the two
//! sides exchange a few frames that carry what SSH itself cannot: the
//! player's real address (every per-IP gate in `mush-net-ssh` keys on it) and
//! a public key whose possession the doorway has already verified.
//!
//! | Doorway sends ([`DoorwayMsg`]) | Instance answers ([`IngressMsg`]) |
//! |---|---|
//! | [`Ping`](DoorwayMsg::Ping) (readiness probe; claims nothing) | [`Pong`](IngressMsg::Pong) |
//! | [`Hello`](DoorwayMsg::Hello) | [`Open`](IngressMsg::Open) / [`Blocked`](IngressMsg::Blocked) / [`Refused`](IngressMsg::Refused) |
//! | [`KeyQuery`](DoorwayMsg::KeyQuery) (zero or more) | [`KeyReply`](IngressMsg::KeyReply) |
//! | [`Begin`](DoorwayMsg::Begin) (until one is `Ready`) | [`Ready`](IngressMsg::Ready) / [`BadCreds`](IngressMsg::BadCreds) / [`Banned`](IngressMsg::Banned) |
//!
//! After `Ready` the stream carries SSH. Passwords never cross the preamble:
//! they travel inside the upstream SSH authentication, so the instance's
//! failure counting is untouched.
//!
//! ## Framing
//!
//! The same shape as `mush-ipc`'s codec: a `u32` big-endian body length, then
//! a self-describing MessagePack body (`{ version, msg }`). Reads use
//! `read_exact` only and never a buffered reader, so **no byte past the frame
//! is consumed** — the SSH stream that follows is intact. A body larger than
//! [`MAX_FRAME`] is refused before any allocation.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

use std::net::SocketAddr;

use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The preamble version both sides speak. A frame carrying any other version
/// is answered with [`IngressMsg::VersionMismatch`] and the stream is closed.
pub const VERSION: u16 = 1;

/// Largest frame body either side accepts (16 KiB). A key line is at most
/// [`MAX_KEY_LEN`], so every legitimate frame fits with room to spare.
pub const MAX_FRAME: usize = 16 * 1024;

/// At most this many [`DoorwayMsg::KeyQuery`] frames per stream.
pub const MAX_KEY_QUERIES: usize = 8;

/// At most this many [`DoorwayMsg::Begin`] frames per stream.
pub const MAX_BEGINS: usize = 4;

/// Longest username either side forwards, in bytes.
pub const MAX_USER_LEN: usize = 256;

/// Longest OpenSSH public-key line either side forwards, in bytes.
pub const MAX_KEY_LEN: usize = 8 * 1024;

/// Frames the doorway sends to the instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoorwayMsg {
    /// A readiness probe. The instance answers [`IngressMsg::Pong`] and
    /// closes; nothing is claimed, so the doorway's own address never takes a
    /// per-IP slot.
    Ping,
    /// The first frame of a player's connection.
    Hello {
        /// The player's address, as the doorway accepted it. Every per-IP
        /// gate in the instance uses this address.
        peer: SocketAddr,
        /// The doorway address the player connected to.
        local: SocketAddr,
        /// The doorway's connection id, for correlating logs.
        conn: u64,
    },
    /// "Would you accept this key for this user?" — SSH's own query phase,
    /// asked without a signature. No failure is counted.
    KeyQuery {
        /// The upstream username (`account` or `account+char`).
        user: String,
        /// The key, as an OpenSSH `"type base64 [comment]"` line.
        key: String,
    },
    /// Ends the preamble once answered [`IngressMsg::Ready`].
    Begin {
        /// The upstream username the SSH session will authenticate as.
        user: String,
        /// A key the doorway verified the player holds (russh checked the
        /// signature downstream). `None` means password or the front door.
        attested_key: Option<String>,
    },
}

/// Frames the instance sends to the doorway.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IngressMsg {
    /// The answer to [`DoorwayMsg::Ping`].
    Pong,
    /// [`DoorwayMsg::Hello`] accepted; the per-IP slot is claimed.
    Open,
    /// The player's address is blocked. `notice` is the words to show them.
    Blocked {
        /// What to tell the player (one line).
        notice: String,
    },
    /// The player's address is over the instance's per-IP connection cap.
    Refused,
    /// The answer to [`DoorwayMsg::KeyQuery`].
    KeyReply {
        /// Whether the instance would accept a signed login with this key.
        acceptable: bool,
    },
    /// [`DoorwayMsg::Begin`] accepted: SSH follows on this stream.
    Ready,
    /// [`DoorwayMsg::Begin`]'s attested key is not registered to the account
    /// (or the account does not exist). The preamble stays open.
    BadCreds,
    /// The account is banned. The preamble stays open, but the doorway
    /// rejects the login.
    Banned,
    /// The doorway spoke a preamble version this instance does not.
    VersionMismatch {
        /// The version this instance speaks.
        supported: u16,
    },
}

/// A preamble failure.
#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    /// The underlying stream failed.
    #[error("preamble i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The stream ended before a frame began.
    #[error("preamble stream closed")]
    Closed,
    /// A frame declared (or would encode) a body over [`MAX_FRAME`].
    #[error("preamble frame of {0} bytes exceeds the {MAX_FRAME}-byte limit")]
    TooLarge(usize),
    /// The body was not a valid frame.
    #[error("preamble decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    /// The body could not be encoded.
    #[error("preamble encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    /// The peer speaks another preamble version.
    #[error("preamble version {0} (this side speaks {VERSION})")]
    Version(u16),
}

#[derive(Serialize)]
struct FrameOut<'a, T> {
    version: u16,
    msg: &'a T,
}

#[derive(Deserialize)]
struct FrameVersion {
    version: u16,
    #[allow(dead_code)]
    msg: IgnoredAny,
}

#[derive(Deserialize)]
struct FrameIn<T> {
    #[allow(dead_code)]
    version: u16,
    msg: T,
}

/// Encode one message as a complete frame (length prefix included).
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtoError> {
    let body = rmp_serde::to_vec_named(&FrameOut {
        version: VERSION,
        msg,
    })?;
    if body.len() > MAX_FRAME {
        return Err(ProtoError::TooLarge(body.len()));
    }
    let len = u32::try_from(body.len()).map_err(|_| ProtoError::TooLarge(body.len()))?;
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode one frame body (no length prefix). The version is checked before
/// the message is decoded, so a newer peer's unknown message shape is reported
/// as [`ProtoError::Version`], not as a decode error.
pub fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, ProtoError> {
    let head: FrameVersion = rmp_serde::from_slice(body)?;
    if head.version != VERSION {
        return Err(ProtoError::Version(head.version));
    }
    let frame: FrameIn<T> = rmp_serde::from_slice(body)?;
    Ok(frame.msg)
}

/// Write one message as a frame and flush.
pub async fn write_msg<W, T>(w: &mut W, msg: &T) -> Result<(), ProtoError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let frame = encode(msg)?;
    w.write_all(&frame).await?;
    w.flush().await?;
    Ok(())
}

/// Read exactly one frame and decode it. Consumes the frame's bytes and
/// nothing more. A stream that ends before the first length byte is
/// [`ProtoError::Closed`]; one that ends inside a frame is an I/O error.
pub async fn read_msg<R, T>(r: &mut R) -> Result<T, ProtoError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < len.len() {
        let Some(rest) = len.get_mut(got..) else {
            break;
        };
        let n = r.read(rest).await?;
        if n == 0 {
            return if got == 0 {
                Err(ProtoError::Closed)
            } else {
                Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into())
            };
        }
        got += n;
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(ProtoError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    decode_body(&body)
}

/// Whether a username is forwardable: non-empty, at most [`MAX_USER_LEN`]
/// bytes, and free of control characters and of the invisible format
/// characters (bidi overrides, zero-width joiners, the BOM) that would
/// otherwise reach log lines, the menu's hints and the instance's name parser.
#[must_use]
pub fn user_is_forwardable(user: &str) -> bool {
    !user.is_empty()
        && user.len() <= MAX_USER_LEN
        && !user
            .chars()
            .any(|c| c.is_control() || is_invisible_format(c))
}

/// The Unicode format characters a username must not carry.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn every_message_round_trips() {
        let doorway = [
            DoorwayMsg::Ping,
            DoorwayMsg::Hello {
                peer: addr("203.0.113.9:50123"),
                local: addr("[2001:db8::10]:22"),
                conn: 7,
            },
            DoorwayMsg::KeyQuery {
                user: "faenix".into(),
                key: "ssh-ed25519 AAAA x".into(),
            },
            DoorwayMsg::Begin {
                user: "faenix+Kira".into(),
                attested_key: Some("ssh-ed25519 AAAA".into()),
            },
            DoorwayMsg::Begin {
                user: "play".into(),
                attested_key: None,
            },
        ];
        for msg in doorway {
            let (mut a, mut b) = tokio::io::duplex(64 * 1024);
            write_msg(&mut a, &msg).await.unwrap();
            let back: DoorwayMsg = read_msg(&mut b).await.unwrap();
            assert_eq!(back, msg);
        }
        let ingress = [
            IngressMsg::Pong,
            IngressMsg::Open,
            IngressMsg::Blocked {
                notice: "no".into(),
            },
            IngressMsg::Refused,
            IngressMsg::KeyReply { acceptable: true },
            IngressMsg::Ready,
            IngressMsg::BadCreds,
            IngressMsg::Banned,
            IngressMsg::VersionMismatch { supported: 1 },
        ];
        for msg in ingress {
            let (mut a, mut b) = tokio::io::duplex(64 * 1024);
            write_msg(&mut a, &msg).await.unwrap();
            let back: IngressMsg = read_msg(&mut b).await.unwrap();
            assert_eq!(back, msg);
        }
    }

    #[tokio::test]
    async fn reading_a_frame_leaves_the_following_bytes_in_the_stream() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        write_msg(&mut a, &IngressMsg::Ready).await.unwrap();
        a.write_all(b"SSH-2.0-rustmush_0.1\r\n").await.unwrap();
        drop(a);
        let msg: IngressMsg = read_msg(&mut b).await.unwrap();
        assert_eq!(msg, IngressMsg::Ready);
        let mut rest = Vec::new();
        b.read_to_end(&mut rest).await.unwrap();
        assert_eq!(rest, b"SSH-2.0-rustmush_0.1\r\n");
    }

    #[tokio::test]
    async fn an_oversized_length_is_refused_before_allocating() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        let err = read_msg::<_, DoorwayMsg>(&mut b).await.unwrap_err();
        assert!(matches!(err, ProtoError::TooLarge(n) if n == u32::MAX as usize));
    }

    #[test]
    fn an_oversized_message_is_refused_on_encode() {
        let msg = DoorwayMsg::KeyQuery {
            user: "a".into(),
            key: "k".repeat(MAX_FRAME + 1),
        };
        assert!(matches!(encode(&msg), Err(ProtoError::TooLarge(_))));
    }

    #[tokio::test]
    async fn eof_before_a_frame_is_closed_and_inside_one_is_an_error() {
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        assert!(matches!(
            read_msg::<_, DoorwayMsg>(&mut b).await,
            Err(ProtoError::Closed)
        ));

        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&[0, 0]).await.unwrap();
        drop(a);
        assert!(matches!(
            read_msg::<_, DoorwayMsg>(&mut b).await,
            Err(ProtoError::Io(_))
        ));

        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&10u32.to_be_bytes()).await.unwrap();
        a.write_all(&[1, 2, 3]).await.unwrap();
        drop(a);
        assert!(matches!(
            read_msg::<_, DoorwayMsg>(&mut b).await,
            Err(ProtoError::Io(_))
        ));
    }

    #[test]
    fn another_version_is_reported_as_such() {
        #[derive(Serialize)]
        struct Future<'a> {
            version: u16,
            msg: &'a str,
        }
        let body = rmp_serde::to_vec_named(&Future {
            version: 2,
            msg: "SomethingNew",
        })
        .unwrap();
        assert!(matches!(
            decode_body::<DoorwayMsg>(&body),
            Err(ProtoError::Version(2))
        ));
    }

    #[test]
    fn garbage_bodies_are_decode_errors_not_panics() {
        for body in [&[][..], &[0xc1], &[0x80], &[0x92, 0x01], &[0xff; 64]] {
            assert!(decode_body::<DoorwayMsg>(body).is_err());
        }
    }

    #[test]
    fn forwardable_users() {
        assert!(user_is_forwardable("faenix+Kira"));
        assert!(!user_is_forwardable(""));
        assert!(!user_is_forwardable("a\nb"));
        assert!(!user_is_forwardable("a\u{1b}[31m"));
        assert!(!user_is_forwardable("dev1+admin\u{202e}"));
        assert!(!user_is_forwardable("fae\u{200b}nix"));
        assert!(user_is_forwardable("Kïra"));
        assert!(!user_is_forwardable(&"x".repeat(MAX_USER_LEN + 1)));
    }
}
