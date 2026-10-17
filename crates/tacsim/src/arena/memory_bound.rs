//! Snapshot memory decoding is bounded before allocating the byte vector, then
//! checked against the largest class reachable from the immutable sealed class.
//! A damaged or dropped mind may lawfully retain keys from its earlier class.
use std::fmt;

use serde::Deserializer;
use serde::de::{Error as _, SeqAccess, Visitor};

use super::snapshot::SnapshotError;
use super::state::Arena;

fn check(bytes: &[u8], keys: usize) -> Result<(), &'static str> {
    // The initial, never-run mind uses an empty byte vector, not encoded `{}`.
    if bytes.is_empty() {
        return Ok(());
    }
    let memory = mush_mind::memory::decode(bytes).ok_or("noncanonical mind memory")?;
    let limits = mush_mind::memory::Limits {
        keys,
        ..super::mind::STOCK_MEMORY_LIMITS
    };
    mush_mind::memory::check(&memory, &limits).map_err(|_| "mind memory exceeds captured class")
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(decoder: D) -> Result<Vec<u8>, D::Error> {
    struct MemoryVisitor;
    impl<'de> Visitor<'de> for MemoryVisitor {
        type Value = Vec<u8>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bounded canonical mind memory bytes")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
            let limit = super::snapshot_capacity::memory_bytes(mush_mind::MAX_MEMORY_ANY_CLASS.keys)
                as usize;
            if seq.size_hint().is_some_and(|size| size > limit) {
                return Err(A::Error::custom("mind memory byte limit"));
            }
            let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(limit));
            while let Some(byte) = seq.next_element::<u8>()? {
                if bytes.len() == limit {
                    return Err(A::Error::custom("mind memory byte limit"));
                }
                if bytes.len() == bytes.capacity() {
                    let capacity = bytes.capacity().saturating_mul(2).max(128).min(limit);
                    bytes
                        .try_reserve_exact(capacity - bytes.len())
                        .map_err(|_| A::Error::custom("mind memory allocation failed"))?;
                }
                bytes.push(byte);
            }
            check(&bytes, mush_mind::MAX_MEMORY_ANY_CLASS.keys).map_err(A::Error::custom)?;
            Ok(bytes)
        }
    }
    decoder.deserialize_seq(MemoryVisitor)
}

pub(crate) fn validate_arena(arena: &Arena) -> Result<(), SnapshotError> {
    for (id, mind) in &arena.minds {
        let unit = arena.unit(*id).ok_or_else(|| {
            SnapshotError::Malformed("mind memory refers to an absent unit".into())
        })?;
        let keys = arena
            .classes
            .rows()
            .iter()
            .filter(|row| row.class <= unit.seal.class)
            .map(|row| row.memory_keys)
            .max()
            .unwrap_or(0);
        check(&mind.memory, keys).map_err(|word| SnapshotError::Malformed(word.into()))?;
        if let Some(pending) = &mind.pending {
            check(&pending.memory, keys).map_err(|word| SnapshotError::Malformed(word.into()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::core::CoreClass;
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_of, pose, tank};
    use mush_core::ObjId;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Wire {
        #[serde(deserialize_with = "super::deserialize")]
        memory: Vec<u8>,
    }
    fn read(bytes: &[u8]) -> Result<Wire, serde_json::Error> {
        serde_json::from_value(serde_json::json!({"memory":bytes}))
    }
    fn memory(keys: usize) -> Vec<u8> {
        let memory = (0..keys)
            .map(|key| {
                (
                    format!("key{key:03}"),
                    mush_mind::memory::Value::Ints(vec![i64::MIN; 32]),
                )
            })
            .collect();
        mush_mind::memory::encode(&memory)
    }
    #[test]
    fn m6_memory_decode_accepts_empty_sentinel_and_maximum_canonical_shape() {
        assert!(read(&[]).unwrap().memory.is_empty());
        assert_eq!(read(b"{}").unwrap().memory, b"{}");
        let bytes = memory(128);
        assert_eq!(read(&bytes).unwrap().memory, bytes);
    }
    #[test]
    fn m6_memory_decode_refuses_noncanonical_and_unbounded_shapes() {
        for bytes in [
            b"{ \"a\":1}".as_slice(),
            b"{\"a\":[[1]]}",
            b"{\"a\":1,\"a\":2}",
            &[0xde, 0xad],
        ] {
            assert!(read(bytes).is_err());
        }
        assert!(read(&memory(129)).is_err());
        let limit = super::super::snapshot_capacity::memory_bytes(128) as usize;
        assert!(read(&vec![b' '; limit + 1]).is_err());
    }
    #[test]
    fn m6_memory_recovery_uses_sealed_reachable_class_after_drop_or_damage() {
        let id = ObjId(1);
        let mut arena = arena_of(vec![tank(id, Team::Red, pose(2, 2))]);
        arena.unit_mut(id).unwrap().seal.class = CoreClass::II;
        // Class-I hardware and a zero effective class must not discard keys
        // lawfully written earlier under the captured Class-II selection.
        let row = super::super::mind::MindState {
            memory: memory(64),
            ..Default::default()
        };
        arena.minds.insert(id, row);
        assert!(validate_arena(&arena).is_ok());
        arena.unit_mut(id).unwrap().seal.class = CoreClass::I;
        assert!(validate_arena(&arena).is_err());
    }
}
