//! The length-prefixed frame codec.
//!
//! Each message on the wire is one frame: a `u32` big-endian body length
//! followed by that many bytes of self-describing MessagePack ([`rmp_serde`]).
//! The length prefix lets a byte-stream reader (M16b's Unix socket) know
//! exactly how many bytes a message needs before decoding, so a partial read is
//! *needs-more-bytes*, never a decode error.
//!
//! - [`encode_frame`] serializes one value into a framed byte buffer.
//! - [`decode_frame`] decodes one frame from the front of a buffer, reporting
//!   how many bytes it consumed, or [`None`] when the buffer does not yet hold a
//!   whole frame.
//! - [`FrameDecoder`] wraps a growable buffer for the streaming case: feed it
//!   arbitrary chunks and pull whole messages out one at a time.
//!
//! The body encoding is self-describing on purpose: the boundary carries
//! `serde_json::Value` payloads ([`mush_core::Inbound::Query`],
//! [`OutputEvent::Panel`](mush_proto::OutputEvent)) whose structure is only
//! known at runtime, which `bincode` cannot represent.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{self, Write};

/// The number of bytes in a frame's length prefix (`u32`, big-endian).
pub const LEN_PREFIX: usize = 4;

/// Largest body a single frame may declare (16 MiB). A length prefix above this
/// is rejected as [`CodecError::FrameTooLarge`] rather than trusted into an
/// allocation — a hostile or corrupt peer must not be able to make the decoder
/// reserve an unbounded buffer.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// A framing or (de)serialization failure.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// The body could not be MessagePack-encoded.
    #[error("ipc encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    /// The body could not be MessagePack-decoded.
    #[error("ipc decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    /// A frame body exceeded [`MAX_FRAME_LEN`]. On decode this is the exact
    /// declared body length; on encode it is the first attempted body length
    /// that crossed the limit (and therefore a lower bound on the complete
    /// encoding, which is deliberately never constructed).
    #[error(
        "ipc frame too large: attempted or declared body length {0} bytes exceeds the {MAX_FRAME_LEN}-byte limit"
    )]
    FrameTooLarge(usize),
}

/// A `Write` adapter that appends one MessagePack body directly to its final
/// frame buffer but refuses the first write that would cross `limit`.
///
/// Serializing into the destination avoids first constructing a second,
/// potentially oversized body `Vec`. The caller rolls the append back on any
/// error, so already-framed bytes remain semantically unchanged.
struct FrameBodyWriter<'a> {
    out: &'a mut Vec<u8>,
    body_start: usize,
    limit: usize,
    overflow_at: Option<usize>,
}

impl FrameBodyWriter<'_> {
    fn body_len(&self) -> usize {
        self.out.len() - self.body_start
    }

    fn overflow_error(&mut self, attempted_len: usize) -> io::Error {
        self.overflow_at.get_or_insert(attempted_len);
        io::Error::new(io::ErrorKind::FileTooLarge, "IPC frame body limit exceeded")
    }
}

impl Write for FrameBodyWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        let attempted_len = self.body_len().saturating_add(buf.len());
        if attempted_len > self.limit {
            return Err(self.overflow_error(attempted_len));
        }
        self.out
            .try_reserve(buf.len())
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        self.out.extend_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serialize `msg` into a length-prefixed frame appended to `out`.
///
/// Fails only if the value is unserializable or its encoded body would exceed
/// [`MAX_FRAME_LEN`] (in which case `out` is left unchanged).
pub fn encode_frame_into<T: Serialize>(msg: &T, out: &mut Vec<u8>) -> Result<(), CodecError> {
    encode_frame_into_capped(msg, out, MAX_FRAME_LEN)
}

/// Inner encoder parameterized only so unit tests can exercise exact boundary
/// behavior without allocating a 16-MiB fixture.
fn encode_frame_into_capped<T: Serialize>(
    msg: &T,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), CodecError> {
    debug_assert!(limit <= u32::MAX as usize);
    let original_len = out.len();
    out.extend_from_slice(&[0; LEN_PREFIX]);
    let body_start = out.len();

    let (encoded, overflow_at) = {
        let mut writer = FrameBodyWriter {
            out,
            body_start,
            limit,
            overflow_at: None,
        };
        let encoded = rmp_serde::encode::write_named(&mut writer, msg);
        (encoded, writer.overflow_at)
    };

    if let Some(attempted_len) = overflow_at {
        out.truncate(original_len);
        return Err(CodecError::FrameTooLarge(attempted_len));
    }
    if let Err(error) = encoded {
        out.truncate(original_len);
        return Err(CodecError::Encode(error));
    }

    let body_len = out.len() - body_start;
    // `limit <= MAX_FRAME_LEN` in production, which fits in a u32.
    let len = body_len as u32;
    out[original_len..body_start].copy_from_slice(&len.to_be_bytes());
    Ok(())
}

/// Serialize `msg` into a fresh length-prefixed frame.
pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    encode_frame_into(msg, &mut out)?;
    Ok(out)
}

/// Try to decode one frame from the front of `buf`.
///
/// Returns:
/// - `Ok(Some((msg, consumed)))` — a whole frame was decoded; `consumed` bytes
///   (prefix + body) should be removed from the front of `buf`.
/// - `Ok(None)` — `buf` does not yet hold a complete frame; read more bytes and
///   retry. A truncated frame is always *needs-more-bytes*, never an error.
/// - `Err(_)` — the length prefix was oversized ([`CodecError::FrameTooLarge`])
///   or a complete body failed to decode.
pub fn decode_frame<T: DeserializeOwned>(buf: &[u8]) -> Result<Option<(T, usize)>, CodecError> {
    if buf.len() < LEN_PREFIX {
        return Ok(None);
    }
    let mut len_bytes = [0u8; LEN_PREFIX];
    len_bytes.copy_from_slice(&buf[..LEN_PREFIX]);
    let body_len = u32::from_be_bytes(len_bytes) as usize;
    if body_len > MAX_FRAME_LEN {
        return Err(CodecError::FrameTooLarge(body_len));
    }
    let frame_end = LEN_PREFIX + body_len;
    if buf.len() < frame_end {
        return Ok(None);
    }
    let msg = rmp_serde::from_slice(&buf[LEN_PREFIX..frame_end])?;
    Ok(Some((msg, frame_end)))
}

/// A streaming frame reader.
///
/// Feed it bytes as they arrive from a transport (any chunking) with [`feed`]
/// and pull decoded messages with [`next_message`], which yields [`None`] once the
/// buffered bytes do not contain another whole frame.
///
/// [`feed`]: FrameDecoder::feed
/// [`next_message`]: FrameDecoder::next_message
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    /// Offset of the first undecoded byte; the buffer is compacted lazily so a
    /// steady stream does not `drain` from the front on every message.
    start: usize,
}

impl FrameDecoder {
    /// A fresh, empty decoder.
    #[must_use]
    pub fn new() -> FrameDecoder {
        FrameDecoder {
            buf: Vec::new(),
            start: 0,
        }
    }

    /// Append freshly received bytes to the internal buffer.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Decode the next whole frame, if the buffer holds one.
    ///
    /// `Ok(None)` means *needs more bytes* — feed more and call again. An error
    /// is a genuine protocol fault (oversized or undecodable body).
    pub fn next_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, CodecError> {
        match decode_frame::<T>(&self.buf[self.start..])? {
            Some((msg, consumed)) => {
                self.start += consumed;
                // Reclaim consumed bytes once they dominate the buffer, keeping
                // memory bounded without paying a shift on every frame.
                if self.start > 4096 && self.start * 2 >= self.buf.len() {
                    self.buf.drain(..self.start);
                    self.start = 0;
                }
                Ok(Some(msg))
            }
            None => Ok(None),
        }
    }

    /// Bytes buffered but not yet decoded (a partial frame in flight).
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.buf.len() - self.start
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use serde::ser::{SerializeSeq, Serializer};

    use super::*;

    /// A streaming `Serialize` implementation: the encoder sees one element
    /// at a time and cannot pre-build the complete body as a hidden fixture.
    struct StringChunks<'a> {
        chunks: &'a [&'a str],
        visited: &'a Cell<usize>,
    }

    impl Serialize for StringChunks<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut seq = serializer.serialize_seq(Some(self.chunks.len()))?;
            for chunk in self.chunks {
                self.visited.set(self.visited.get() + 1);
                seq.serialize_element(chunk)?;
            }
            seq.end()
        }
    }

    #[test]
    fn capped_encoder_accepts_an_exact_limit_body() {
        // fixarray(2) + fixstr(2) + "ab" + fixstr(3) + "cde" = 8 bytes.
        let visited = Cell::new(0);
        let value = StringChunks {
            chunks: &["ab", "cde"],
            visited: &visited,
        };
        let mut out = vec![0xaa, 0xbb];

        encode_frame_into_capped(&value, &mut out, 8).unwrap();

        assert_eq!(&out[..2], &[0xaa, 0xbb]);
        assert_eq!(u32::from_be_bytes(out[2..6].try_into().unwrap()), 8);
        assert_eq!(out.len(), 2 + LEN_PREFIX + 8);
        assert_eq!(visited.get(), 2);
    }

    #[test]
    fn capped_encoder_rolls_back_the_destination_on_first_oversized_write() {
        // The first two elements exactly fill the 8-byte limit; serializing
        // the third must fail immediately and leave the pre-existing stream
        // byte-for-byte unchanged.
        let visited = Cell::new(0);
        let value = StringChunks {
            chunks: &["ab", "cde", "never appended"],
            visited: &visited,
        };
        let mut out = vec![0xaa, 0xbb, 0xcc];
        let before = out.clone();

        let error = encode_frame_into_capped(&value, &mut out, 8).unwrap_err();

        assert!(matches!(error, CodecError::FrameTooLarge(n) if n > 8));
        assert_eq!(out, before);
        assert_eq!(visited.get(), 3, "serialization stopped at the overflow");
    }

    #[test]
    fn ordinary_encode_errors_also_roll_back_the_destination() {
        struct FailsAfterOneElement;

        impl Serialize for FailsAfterOneElement {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("written")?;
                Err(<S::Error as serde::ser::Error>::custom(
                    "intentional test failure",
                ))
            }
        }

        let mut out = vec![1, 2, 3, 4];
        let before = out.clone();
        let error = encode_frame_into(&FailsAfterOneElement, &mut out).unwrap_err();

        assert!(matches!(error, CodecError::Encode(_)));
        assert_eq!(out, before);
    }
}
