//! Durable key provisioning for opaque K5 legal-action offers.

use std::path::{Path, PathBuf};

use mush_core::{MAX_OFFER_SECRET_VERSIONS, OfferKeyRing, OfferSecret, OfferSecretError};

use crate::durable_keys::{DurableKeyStore, DurableKeyStoreError, KeyFamily, VersionedKeyMaterial};

struct OfferKeyFamily;

impl KeyFamily for OfferKeyFamily {
    type Ring = OfferKeyRing;
    type RingError = OfferSecretError;

    const MAX_VERSIONS: usize = MAX_OFFER_SECRET_VERSIONS;
    const KEY_PREFIX: &'static str = "key-";
    const MANIFEST_PREFIX: &'static str = "ring-";

    fn build_ring(
        current: VersionedKeyMaterial,
        retained: Vec<VersionedKeyMaterial>,
    ) -> Result<Self::Ring, Self::RingError> {
        OfferKeyRing::new(
            OfferSecret::new(current.version, current.key)?,
            retained
                .into_iter()
                .map(|material| OfferSecret::new(material.version, material.key))
                .collect::<Result<Vec<_>, _>>()?,
        )
    }
}

/// Fail-closed durable offer-key error. No variant carries secret bytes.
pub type OfferKeyStoreError = DurableKeyStoreError<OfferSecretError>;

/// A private, append-only deployment key directory for offer signatures.
#[derive(Clone, Debug)]
pub struct DurableOfferKeyStore {
    inner: DurableKeyStore<OfferKeyFamily>,
}

impl DurableOfferKeyStore {
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

    pub fn load_or_create(&self) -> Result<OfferKeyRing, OfferKeyStoreError> {
        self.inner.load_or_create()
    }

    pub fn rotate(&self) -> Result<OfferKeyRing, OfferKeyStoreError> {
        self.inner.rotate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> DurableOfferKeyStore {
        let nonce: u64 = rand::random();
        DurableOfferKeyStore::new(std::env::temp_dir().join(format!(
            "rustmush-offer-keys-{tag}-{}-{nonce}",
            std::process::id()
        )))
    }

    fn remove(store: &DurableOfferKeyStore) {
        let _ = std::fs::remove_dir_all(store.directory());
    }

    #[test]
    fn first_boot_provisions_and_restart_reuses_exact_key() {
        let store = temp_store("restart");
        let first = store.load_or_create().unwrap();
        let key_path = store.inner.key_path(1);
        let before = std::fs::read(&key_path).unwrap();
        let second = store.load_or_create().unwrap();
        let after = std::fs::read(&key_path).unwrap();
        assert_eq!(first.current_version(), 1);
        assert_eq!(second.current_version(), 1);
        assert_eq!(before, after);
        assert!(!format!("{first:?}").contains(std::str::from_utf8(&before).unwrap().trim()));
        remove(&store);
    }

    #[test]
    fn rotation_is_bounded_and_prunes_retired_secret_files() {
        let store = temp_store("rotate");
        store.load_or_create().unwrap();
        for expected in 2..=5 {
            assert_eq!(store.rotate().unwrap().current_version(), expected);
        }
        assert!(!store.inner.key_path(1).exists());
        for retained in 2..=5 {
            assert!(store.inner.key_path(retained).exists());
        }
        assert_eq!(store.load_or_create().unwrap().current_version(), 5);
        remove(&store);
    }

    #[test]
    fn orphan_successor_key_is_reused_after_pre_manifest_crash() {
        let store = temp_store("orphan");
        store.load_or_create().unwrap();
        store.inner.create_or_recover_key(2).unwrap();
        let orphan = std::fs::read(store.inner.key_path(2)).unwrap();
        assert_eq!(store.rotate().unwrap().current_version(), 2);
        assert_eq!(std::fs::read(store.inner.key_path(2)).unwrap(), orphan);
        remove(&store);
    }

    #[test]
    fn corrupt_highest_manifest_fails_closed() {
        use std::io::Write as _;

        let store = temp_store("corrupt");
        store.load_or_create().unwrap();
        let path = store.inner.manifest_path(2);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path).unwrap();
        file.write_all(b"{").unwrap();
        file.sync_all().unwrap();
        assert!(matches!(
            store.load_or_create(),
            Err(OfferKeyStoreError::Manifest { .. })
        ));
        remove(&store);
    }
}
