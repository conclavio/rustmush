//! Preparation-worker-only immutable tape sharing. Each checkout still passes
//! its own authorization/pin through verify_with; the cache grants no access.
use super::{BulkError, invalid};
use crate::{
    match_::mind_tape::{TapeCheckout, TapeLease},
    minds::resimulation::VerifiedTape,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

const KEYS: usize = 64;
type TapeKey = ([u8; 32], u64);
type TapeEntries = BTreeMap<TapeKey, Weak<VerifiedTape>>;

#[derive(Default)]
pub(crate) struct TapeCache {
    entries: Mutex<TapeEntries>,
}
impl TapeCache {
    pub fn verify(&self, checkout: TapeCheckout) -> Result<TapeLease, BulkError> {
        let key = (checkout.artifact_digest(), checkout.encoded_bytes());
        let cached = self
            .entries
            .lock()
            .map_err(|_| invalid("tape cache unavailable"))?
            .get(&key)
            .and_then(Weak::upgrade);
        let lease = checkout
            .verify_with(cached)
            .map_err(|e| invalid(e.to_string()))?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| invalid("tape cache unavailable"))?;
        entries.retain(|_, value| value.strong_count() != 0);
        if !entries.contains_key(&key) && entries.len() >= KEYS {
            // Weak-key eviction cannot invalidate a held replay or its pin.
            entries.pop_first();
        }
        entries.insert(key, Arc::downgrade(&lease.tape));
        Ok(lease)
    }
}
