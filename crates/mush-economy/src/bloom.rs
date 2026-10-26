//! The key filter (doc 13 §2.3, W-D3): a **scalable Bloom filter** over every
//! idempotency key ever posted.
//!
//! A negative is certain — the key was never posted — so a post whose key the
//! RAM window does not hold may proceed at once when the filter says so. A
//! positive is a *maybe*: the key is in the window, or on disk under
//! `idem:{h}`, or the filter is wrong about it, at a rate near 1 in 10 000 at
//! its design load. Only that case takes the slow lookup.
//!
//! **Scalable**: a sequence of slices, each a plain Bloom filter sized for a
//! capacity that doubles from [`FIRST_SLICE_CAPACITY`], at [`BITS_PER_KEY`]
//! bits a key and [`HASHES`] hash functions. A lookup asks every slice; an
//! insert goes to the newest, and a new slice opens when it fills. Nothing is
//! ever rebuilt or resized, nothing is ever deleted (D-12-5: a key is never
//! forgotten), and the whole thing is ~2.5 MB per million keys.
//!
//! **Deterministic across machines and boots**: `SHA-256(namespace ‖ 0x00 ‖
//! key)` gives two 64-bit halves `h1`, `h2`, and the i-th index is
//! `(h1 + i·h2) mod m` (Kirsch–Mitzenmacher double hashing). The bytes ride in
//! a checkpoint part and are rebuilt whole by a full replay.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::record::IdempotencyKey;

/// Bits per key each slice is sized for (~1e-4 false positives at capacity
/// with [`HASHES`] hashes).
pub const BITS_PER_KEY: u64 = 20;
/// Hash functions per lookup (the optimum for 20 bits a key is ~14).
pub const HASHES: u32 = 14;
/// The first slice's capacity; each next slice doubles.
pub const FIRST_SLICE_CAPACITY: u64 = 65_536;
/// [`ScalableBloom::format`] when written by this code.
pub const BLOOM_FORMAT: u32 = 1;

/// One slice: a plain Bloom filter of `m` bits sized for `capacity` keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    /// Keys this slice is sized for.
    pub capacity: u64,
    /// Keys inserted into this slice.
    pub count: u64,
    /// The bit array, as hex (serde-friendly; ~5 MB per million keys in a
    /// document, 2.5 MB in RAM).
    #[serde(with = "hex_words")]
    pub bits: Vec<u64>,
}

impl Slice {
    fn new(capacity: u64) -> Slice {
        let m = capacity.saturating_mul(BITS_PER_KEY).max(64);
        let words = usize::try_from(m.div_ceil(64)).unwrap_or(usize::MAX);
        Slice {
            capacity,
            count: 0,
            bits: vec![0u64; words],
        }
    }

    fn m(&self) -> u64 {
        u64::try_from(self.bits.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(64)
    }

    fn set(&mut self, h1: u64, h2: u64) {
        let m = self.m();
        for i in 0..u64::from(HASHES) {
            let idx = h1.wrapping_add(i.wrapping_mul(h2)) % m;
            let word = usize::try_from(idx / 64).unwrap_or(0);
            self.bits[word] |= 1u64 << (idx % 64);
        }
    }

    fn get(&self, h1: u64, h2: u64) -> bool {
        let m = self.m();
        (0..u64::from(HASHES)).all(|i| {
            let idx = h1.wrapping_add(i.wrapping_mul(h2)) % m;
            let word = usize::try_from(idx / 64).unwrap_or(0);
            self.bits[word] & (1u64 << (idx % 64)) != 0
        })
    }
}

/// The filter: every key ever posted, as slices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScalableBloom {
    /// [`BLOOM_FORMAT`].
    pub format: u32,
    /// The slices, oldest first; inserts go to the last.
    pub slices: Vec<Slice>,
}

impl Default for ScalableBloom {
    fn default() -> Self {
        ScalableBloom::new()
    }
}

impl ScalableBloom {
    /// Prepare one exact insertion without cloning the filter or publishing a
    /// bit. The bounded patch contains at most HASHES changed words.
    pub(crate) fn prepare_insert(&self, key: &IdempotencyKey) -> Option<PreparedBloomInsert> {
        if !self.is_well_formed() {
            return None;
        }
        let (h1, h2) = Self::hashes(key);
        if self.slices.iter().any(|slice| slice.get(h1, h2)) {
            return Some(PreparedBloomInsert::Present);
        }
        let last = self.slices.last()?;
        if last.count == last.capacity {
            let capacity = last.capacity.checked_mul(2)?;
            let mut slice = Slice::new(capacity);
            slice.set(h1, h2);
            slice.count = 1;
            return Some(PreparedBloomInsert::Append {
                length: self.slices.len(),
                capacity: last.capacity,
                count: last.count,
                slice,
            });
        }
        let mut words = std::collections::BTreeMap::new();
        for i in 0..u64::from(HASHES) {
            let index = h1.wrapping_add(i.wrapping_mul(h2)) % last.m();
            let word = usize::try_from(index / 64).ok()?;
            let old = *last.bits.get(word)?;
            let entry = words.entry(word).or_insert((old, old));
            entry.1 |= 1u64 << (index % 64);
        }
        Some(PreparedBloomInsert::Words {
            length: self.slices.len(),
            capacity: last.capacity,
            count: last.count,
            next_count: last.count.checked_add(1)?,
            words: words.into_iter().collect(),
        })
    }

    /// All lookup/shape checks finish before mutable word references escape.
    pub(crate) fn bind_insert(
        &mut self,
        prepared: PreparedBloomInsert,
    ) -> Option<BoundBloomInsert<'_>> {
        match prepared {
            PreparedBloomInsert::Present => Some(BoundBloomInsert::Present),
            PreparedBloomInsert::Append {
                length,
                capacity,
                count,
                slice,
            } => {
                let last = self.slices.last()?;
                if self.slices.len() != length || last.capacity != capacity || last.count != count {
                    return None;
                }
                Some(BoundBloomInsert::Append {
                    slices: &mut self.slices,
                    slice,
                })
            }
            PreparedBloomInsert::Words {
                length,
                capacity,
                count,
                next_count,
                words,
            } => {
                if self.slices.len() != length {
                    return None;
                }
                let last = self.slices.last_mut()?;
                if last.capacity != capacity
                    || last.count != count
                    || words
                        .iter()
                        .any(|(index, (old, _))| last.bits.get(*index) != Some(old))
                {
                    return None;
                }
                let mut remaining = last.bits.as_mut_slice();
                let mut offset = 0;
                let mut bound = Vec::with_capacity(words.len());
                for (index, (_, value)) in words {
                    let relative = index.checked_sub(offset)?;
                    if relative >= remaining.len() {
                        return None;
                    }
                    let (_, right) = remaining.split_at_mut(relative);
                    let (word, rest) = right.split_first_mut()?;
                    bound.push((word, value));
                    remaining = rest;
                    offset = index.checked_add(1)?;
                }
                Some(BoundBloomInsert::Words {
                    words: bound,
                    count: &mut last.count,
                    next_count,
                })
            }
        }
    }
    /// An empty filter with its first slice.
    #[must_use]
    pub fn new() -> ScalableBloom {
        ScalableBloom {
            format: BLOOM_FORMAT,
            slices: vec![Slice::new(FIRST_SLICE_CAPACITY)],
        }
    }

    /// The two 64-bit hashes of a key, the same on every machine.
    #[must_use]
    pub fn hashes(key: &IdempotencyKey) -> (u64, u64) {
        let mut hasher = Sha256::new();
        hasher.update(key.namespace.as_bytes());
        hasher.update([0u8]);
        hasher.update(key.key.as_bytes());
        let digest = hasher.finalize();
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        a.copy_from_slice(&digest[..8]);
        b.copy_from_slice(&digest[8..16]);
        // An even `h2` would visit only half the bits; make it odd.
        (u64::from_le_bytes(a), u64::from_le_bytes(b) | 1)
    }

    /// Remember a key. Idempotent.
    pub fn insert(&mut self, key: &IdempotencyKey) {
        let (h1, h2) = Self::hashes(key);
        if self.slices.iter().any(|s| s.get(h1, h2)) {
            return;
        }
        let last = self.slices.len() - 1;
        if self.slices[last].count >= self.slices[last].capacity {
            let next = self.slices[last].capacity.saturating_mul(2);
            self.slices.push(Slice::new(next));
        }
        let slice = self.slices.last_mut().expect("at least one slice");
        slice.set(h1, h2);
        slice.count = slice.count.saturating_add(1);
    }

    /// Whether a key **may** have been posted: `false` is certain, `true` is a
    /// maybe (the window, the store, or a false positive).
    #[must_use]
    pub fn might_contain(&self, key: &IdempotencyKey) -> bool {
        let (h1, h2) = Self::hashes(key);
        self.slices.iter().any(|s| s.get(h1, h2))
    }

    /// Keys inserted (distinct at insertion time).
    #[must_use]
    pub fn len(&self) -> u64 {
        self.slices.iter().map(|s| s.count).sum()
    }

    /// No key inserted yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes of bit array in RAM.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.slices.iter().map(|s| s.bits.len() * 8).sum()
    }

    /// Whether the filter is at a format this code reads and every slice is
    /// well formed (a non-empty bit array, a count no greater than its
    /// capacity).
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.format == BLOOM_FORMAT
            && !self.slices.is_empty()
            && self
                .slices
                .iter()
                .all(|s| !s.bits.is_empty() && s.count <= s.capacity && s.capacity > 0)
    }
}

#[derive(Debug)]
pub(crate) enum PreparedBloomInsert {
    Present,
    Append {
        length: usize,
        capacity: u64,
        count: u64,
        slice: Slice,
    },
    Words {
        length: usize,
        capacity: u64,
        count: u64,
        next_count: u64,
        words: Vec<(usize, (u64, u64))>,
    },
}

pub(crate) enum BoundBloomInsert<'a> {
    Present,
    Append {
        slices: &'a mut Vec<Slice>,
        slice: Slice,
    },
    Words {
        words: Vec<(&'a mut u64, u64)>,
        count: &'a mut u64,
        next_count: u64,
    },
}
impl BoundBloomInsert<'_> {
    pub(crate) fn apply(self) {
        match self {
            Self::Present => {}
            Self::Append { slices, slice } => slices.push(slice),
            Self::Words {
                words,
                count,
                next_count,
            } => {
                for (word, value) in words {
                    *word = value;
                }
                *count = next_count;
            }
        }
    }
}

/// `Vec<u64>` as one lowercase hex string, 16 digits a word, little-endian
/// word order.
mod hex_words {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(words: &[u64], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(words.len() * 16);
        for w in words {
            use std::fmt::Write as _;
            let _ = write!(out, "{w:016x}");
        }
        out.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() % 16 != 0 {
            return Err(D::Error::custom("bit array hex is not whole words"));
        }
        text.as_bytes()
            .chunks(16)
            .map(|c| {
                let s = std::str::from_utf8(c).map_err(D::Error::custom)?;
                u64::from_str_radix(s, 16).map_err(D::Error::custom)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u64) -> IdempotencyKey {
        IdempotencyKey::new("t", format!("k-{n}"))
    }

    #[test]
    fn a_key_inserted_is_always_positive_and_a_fresh_filter_says_never() {
        let mut f = ScalableBloom::new();
        assert!(f.is_empty());
        assert!(!f.might_contain(&key(1)));
        for n in 0..10_000 {
            f.insert(&key(n));
        }
        assert_eq!(f.len(), 10_000);
        for n in 0..10_000 {
            assert!(f.might_contain(&key(n)), "{n}");
        }
        f.insert(&key(7));
        assert_eq!(f.len(), 10_000, "a repeat inserts nothing");
        assert!(f.is_well_formed());
    }

    /// F: the false-positive rate at design load is near 1e-4 — under 3e-4
    /// over 100 000 never-inserted keys against 100 000 inserted ones, which
    /// spans the first two slices (65 536 + the next).
    #[test]
    fn the_false_positive_rate_is_near_one_in_ten_thousand() {
        let mut f = ScalableBloom::new();
        for n in 0..100_000 {
            f.insert(&key(n));
        }
        assert_eq!(f.slices.len(), 2, "the second slice opened");
        let positives = (1_000_000..1_100_000)
            .filter(|n| f.might_contain(&key(*n)))
            .count();
        assert!(positives <= 30, "false positives: {positives} of 100 000");
        assert!(
            f.byte_len() <= 600_000,
            "{} bytes for 100 000 keys",
            f.byte_len()
        );
    }

    #[test]
    fn the_filter_round_trips_through_json_and_hashes_the_same() {
        let mut f = ScalableBloom::new();
        for n in 0..1_000 {
            f.insert(&key(n));
        }
        let text = serde_json::to_string(&f).unwrap();
        let back: ScalableBloom = serde_json::from_str(&text).unwrap();
        assert_eq!(back, f);
        assert!(back.might_contain(&key(999)));
        assert!(!back.might_contain(&key(1_000_001)));
        assert_eq!(
            ScalableBloom::hashes(&key(3)),
            ScalableBloom::hashes(&key(3))
        );
        assert_ne!(
            ScalableBloom::hashes(&key(3)),
            ScalableBloom::hashes(&key(4))
        );
        assert_eq!(ScalableBloom::hashes(&key(3)).1 % 2, 1, "h2 is odd");
    }

    #[test]
    fn a_malformed_filter_is_not_well_formed() {
        let mut f = ScalableBloom::new();
        f.slices[0].count = f.slices[0].capacity + 1;
        assert!(!f.is_well_formed());
        let mut g = ScalableBloom::new();
        g.format = 9;
        assert!(!g.is_well_formed());
        let mut h = ScalableBloom::new();
        h.slices.clear();
        assert!(!h.is_well_formed());
    }
}
