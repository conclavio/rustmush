//! Exact resident document accounting; no second quota or lower M6 ceiling.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollUsage {
    pub(crate) records: usize,
    pub(crate) document_bytes: u64,
    pub(crate) key_bytes: u64,
}
impl RollUsage {
    pub const fn records(self) -> usize {
        self.records
    }
    /// Canonical stored JSON values, excluding storage-engine framing.
    pub const fn document_bytes(self) -> u64 {
        self.document_bytes
    }
    pub const fn key_bytes(self) -> u64 {
        self.key_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageChange {
    pub(crate) before: RollUsage,
    pub(crate) after: RollUsage,
}
impl UsageChange {
    pub const fn before(self) -> RollUsage {
        self.before
    }
    pub const fn after(self) -> RollUsage {
        self.after
    }
}

/// Count through the actual JSON serializer without allocating another copy.
/// Values have already passed the owner's bounded serializer on mutation.
pub(crate) fn encoded_len(value: &impl Serialize) -> Option<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|n| *n <= crate::MAX_DOCUMENT_BYTES)
                .ok_or_else(|| std::io::Error::other("roll document bound"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.0)
}
