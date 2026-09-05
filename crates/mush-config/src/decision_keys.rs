//! Durable, restart-stable HMAC keys for K7 delayed-decision prompts.

use std::path::{Path, PathBuf};

use mush_core::{
    DecisionKeyRing, DecisionSecret, DecisionSecretError, MAX_DECISION_SECRET_VERSIONS,
};

use crate::durable_keys::{DurableKeyStore, DurableKeyStoreError, KeyFamily, VersionedKeyMaterial};

struct DecisionKeyFamily;

impl KeyFamily for DecisionKeyFamily {
    type Ring = DecisionKeyRing;
    type RingError = DecisionSecretError;

    const MAX_VERSIONS: usize = MAX_DECISION_SECRET_VERSIONS;
    const KEY_PREFIX: &'static str = "decision-key-";
    const MANIFEST_PREFIX: &'static str = "decision-ring-";

    fn build_ring(
        current: VersionedKeyMaterial,
        retained: Vec<VersionedKeyMaterial>,
    ) -> Result<Self::Ring, Self::RingError> {
        DecisionKeyRing::new(
            DecisionSecret::new(current.version, current.key)?,
            retained
                .into_iter()
                .map(|material| DecisionSecret::new(material.version, material.key))
                .collect::<Result<Vec<_>, _>>()?,
        )
    }
}

/// Fail-closed durable decision-key error. No variant carries secret bytes.
pub type DecisionKeyStoreError = DurableKeyStoreError<DecisionSecretError>;

#[derive(Clone, Debug)]
pub struct DurableDecisionKeyStore {
    inner: DurableKeyStore<DecisionKeyFamily>,
}

impl DurableDecisionKeyStore {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            inner: DurableKeyStore::new(directory),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        self.inner.directory()
    }

    pub fn load_or_create(&self) -> Result<DecisionKeyRing, DecisionKeyStoreError> {
        self.inner.load_or_create()
    }

    pub fn rotate(&self) -> Result<DecisionKeyRing, DecisionKeyStoreError> {
        self.inner.rotate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_reuses_exact_decision_secret_and_family_is_independent() {
        let directory = std::env::temp_dir().join(format!(
            "rustmush-decision-keys-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = DurableDecisionKeyStore::new(&directory);
        let first = store.load_or_create().unwrap();
        let before = std::fs::read(store.inner.key_path(1)).unwrap();
        let second = store.load_or_create().unwrap();
        assert_eq!(first.current_version(), 1);
        assert_eq!(second.current_version(), 1);
        assert_eq!(std::fs::read(store.inner.key_path(1)).unwrap(), before);

        let offers = crate::offer_keys::DurableOfferKeyStore::new(&directory);
        offers.load_or_create().unwrap();
        assert_eq!(store.rotate().unwrap().current_version(), 2);
        assert_eq!(offers.load_or_create().unwrap().current_version(), 1);
        let _ = std::fs::remove_dir_all(directory);
    }
}
