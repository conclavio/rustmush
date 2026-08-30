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
    /// A frame declared a body larger than [`MAX_FRAME_LEN`].
    #[error("ipc frame too large: {0} bytes exceeds the {MAX_FRAME_LEN}-byte limit")]
    FrameTooLarge(usize),
}

/// Serialize `msg` into a length-prefixed frame appended to `out`.
///
/// Fails only if the value is unserializable or its encoded body would exceed
/// [`MAX_FRAME_LEN`] (in which case `out` is left unchanged).
pub fn encode_frame_into<T: Serialize>(msg: &T, out: &mut Vec<u8>) -> Result<(), CodecError> {
    let body = rmp_serde::to_vec_named(msg)?;
    if body.len() > MAX_FRAME_LEN {
        return Err(CodecError::FrameTooLarge(body.len()));
    }
    // `MAX_FRAME_LEN` fits in a u32, so this cast never truncates.
    let len = body.len() as u32;
    out.reserve(LEN_PREFIX + body.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&body);
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
pub fn decode_frame<T: DeserializeOwned>(
    buf: &[u8],
) -> Result<Option<(T, usize)>, CodecError> {
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
