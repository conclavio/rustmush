//! Durable, restart-stable root keys for K4 authoritative resolution RNG.

use std::path::{Path, PathBuf};

use mush_core::{
    MAX_RESOLUTION_SECRET_VERSIONS, ResolutionKeyRing, ResolutionSecret, ResolutionSecretError,
};

use crate::durable_keys::{DurableKeyStore, DurableKeyStoreError, KeyFamily, VersionedKeyMaterial};

struct ResolutionKeyFamily;

impl KeyFamily for ResolutionKeyFamily {
    type Ring = ResolutionKeyRing;
    type RingError = ResolutionSecretError;

    const MAX_VERSIONS: usize = MAX_RESOLUTION_SECRET_VERSIONS;
    const KEY_PREFIX: &'static str = "resolution-key-";
    const MANIFEST_PREFIX: &'static str = "resolution-ring-";

    fn build_ring(
        current: VersionedKeyMaterial,
        retained: Vec<VersionedKeyMaterial>,
    ) -> Result<Self::Ring, Self::RingError> {
        ResolutionKeyRing::new(
            ResolutionSecret::new(current.version, current.key)?,
            retained
                .into_iter()
                .map(|material| ResolutionSecret::new(material.version, material.key))
                .collect::<Result<Vec<_>, _>>()?,
        )
    }
}

/// Fail-closed durable resolution-key error. No variant carries secret bytes.
pub type ResolutionKeyStoreError = DurableKeyStoreError<ResolutionSecretError>;

/// A private, append-only deployment key directory for authoritative RNG.
#[derive(Clone, Debug)]
pub struct DurableResolutionKeyStore {
    inner: DurableKeyStore<ResolutionKeyFamily>,
}

impl DurableResolutionKeyStore {
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

    pub fn load_or_create(&self) -> Result<ResolutionKeyRing, ResolutionKeyStoreError> {
        self.inner.load_or_create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> DurableResolutionKeyStore {
        let nonce: u64 = rand::random();
        DurableResolutionKeyStore::new(std::env::temp_dir().join(format!(
            "rustmush-resolution-keys-{tag}-{}-{nonce}",
            std::process::id()
        )))
    }

    fn remove(store: &DurableResolutionKeyStore) {
        let _ = std::fs::remove_dir_all(store.directory());
    }

    #[test]
    fn restart_reuses_exact_resolution_secret() {
        let store = temp_store("restart");
        let first = store.load_or_create().unwrap();
        let key_path = store.inner.key_path(1);
        let before = std::fs::read(&key_path).unwrap();
        let second = store.load_or_create().unwrap();
        assert_eq!(first.current_version(), 1);
        assert_eq!(second.current_version(), 1);
        assert_eq!(std::fs::read(key_path).unwrap(), before);
        assert!(!format!("{first:?}").contains(std::str::from_utf8(&before).unwrap().trim()));
        remove(&store);
    }

    #[test]
    fn protocol_filenames_cannot_alias_even_in_one_directory() {
        let resolution = temp_store("independent");
        let offer = crate::offer_keys::DurableOfferKeyStore::new(resolution.directory());
        resolution.load_or_create().unwrap();
        offer.load_or_create().unwrap();
        resolution.inner.rotate().unwrap();
        assert_eq!(resolution.load_or_create().unwrap().current_version(), 2);
        assert_eq!(offer.load_or_create().unwrap().current_version(), 1);
        remove(&resolution);
    }
}
