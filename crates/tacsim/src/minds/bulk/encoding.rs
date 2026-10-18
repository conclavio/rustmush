//! Bounded serialization before allocation can exceed a retained artifact cap.
use super::{BulkError, invalid};
use std::io::{self, Write};
struct Bounded {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("artifact byte bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>, BulkError> {
    let mut out = Bounded {
        bytes: Vec::with_capacity(limit.min(4096)),
        limit,
    };
    serde_json::to_writer(&mut out, value)
        .map_err(|_| invalid("artifact serialization or byte bound"))?;
    Ok(out.bytes)
}
/// Compress at the preparation boundary into a writer that cannot grow past
/// the artifact bound. The caller separately bounds and hashes the raw bytes.
pub(super) fn zlib(raw: &[u8], limit: usize) -> Result<Vec<u8>, BulkError> {
    let out = Bounded {
        bytes: Vec::with_capacity(limit.min(4096)),
        limit,
    };
    // The stock100 6v6 probe measured far less repeated snapshot data at
    // default than fast compression. Raw and encoded caps remain unchanged.
    let mut encoder = flate2::write::ZlibEncoder::new(out, flate2::Compression::default());
    encoder
        .write_all(raw)
        .map_err(|_| invalid("frozen proving compression bound"))?;
    Ok(encoder
        .finish()
        .map_err(|_| invalid("frozen proving compression bound"))?
        .bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_during_write_without_growing_past_limit() {
        assert_eq!(encode(&"abcd", 6).expect("six JSON bytes"), b"\"abcd\"");
        assert!(encode(&"abcd", 5).is_err());
        let mut out = Bounded {
            bytes: Vec::new(),
            limit: 2,
        };
        assert!(out.write_all(&[0; 1024]).is_err());
        assert!(out.bytes.is_empty());
    }
}
