//! The geology key family — the fourth durable key family (doc 20 cut SM.1; §3.2, Q-20-12, §16).
//!
//! Surface mining's ground is HMAC-SHA256 under one deployment secret
//! ([`mush_core::GeologySecret`]). It lives in its own private directory
//! (`[tacsim.geology].key_dir`, `geology-keys/` in the shipped template), on the same crash-safe
//! storage as the offer, resolution and decision families, with two rules of its own:
//!
//! - **one version, never rotated** (D-20-4): the family keeps exactly one key
//!   ([`mush_core::MAX_GEOLOGY_SECRET_VERSIONS`] is 1, so a manifest naming a retained version is
//!   refused) and this wrapper has no rotate — a new key would move every reserve on every body;
//! - **a lost key is a disabled layer, not a corrupted one** (§16): the host loads it without `?`
//!   and hands a failure to the game crate as a disabled state. Back the directory up with the
//!   store: a store restored without its matching key reads mining disabled until the directory
//!   is restored too, because the world pins the key's fingerprint at its first keyed boot.

use std::path::{Path, PathBuf};

use mush_core::{GeologySecret, GeologySecretError, MAX_GEOLOGY_SECRET_VERSIONS};

use crate::durable_keys::{DurableKeyStore, DurableKeyStoreError, KeyFamily, VersionedKeyMaterial};

struct GeologyKeyFamily;

impl KeyFamily for GeologyKeyFamily {
    type Ring = GeologySecret;
    type RingError = GeologySecretError;

    const MAX_VERSIONS: usize = MAX_GEOLOGY_SECRET_VERSIONS;
    const KEY_PREFIX: &'static str = "geology-key-";
    const MANIFEST_PREFIX: &'static str = "geology-ring-";

    fn build_ring(
        current: VersionedKeyMaterial,
        retained: Vec<VersionedKeyMaterial>,
    ) -> Result<Self::Ring, Self::RingError> {
        // The manifest validator already refuses a retained list at MAX_VERSIONS = 1; this is the
        // ring's own refusal, so the rule never rests on one check alone.
        if !retained.is_empty() {
            return Err(GeologySecretError::Retained);
        }
        GeologySecret::new(current.version, current.key)
    }
}

/// Fail-closed durable geology-key error. No variant carries secret bytes.
pub type GeologyKeyStoreError = DurableKeyStoreError<GeologySecretError>;

/// The private, one-key deployment directory behind the geology function.
#[derive(Clone, Debug)]
pub struct DurableGeologyKeyStore {
    inner: DurableKeyStore<GeologyKeyFamily>,
}

impl DurableGeologyKeyStore {
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

    /// The directory's one key, provisioning version 1 on first use. There is deliberately no
    /// rotate (D-20-4: the key is never rotated).
    pub fn load_or_create(&self) -> Result<GeologySecret, GeologyKeyStoreError> {
        self.inner.load_or_create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> DurableGeologyKeyStore {
        let nonce: u64 = rand::random();
        DurableGeologyKeyStore::new(std::env::temp_dir().join(format!(
            "rustmush-geology-keys-{tag}-{}-{nonce}",
            std::process::id()
        )))
    }

    fn remove(store: &DurableGeologyKeyStore) {
        let _ = std::fs::remove_dir_all(store.directory());
    }

    #[test]
    fn restart_reuses_the_exact_geology_secret() {
        let store = temp_store("restart");
        let first = store.load_or_create().unwrap();
        let key_path = store.inner.key_path(1);
        let before = std::fs::read(&key_path).unwrap();
        let second = store.load_or_create().unwrap();
        assert_eq!(first.version(), 1);
        assert_eq!(second.version(), 1);
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(std::fs::read(key_path).unwrap(), before);
        remove(&store);
    }

    #[test]
    fn the_geology_family_is_independent_in_a_shared_directory() {
        let geology = temp_store("independent");
        let fingerprint = geology.load_or_create().unwrap().fingerprint();
        let offer = crate::offer_keys::DurableOfferKeyStore::new(geology.directory());
        let resolution =
            crate::resolution_keys::DurableResolutionKeyStore::new(geology.directory());
        offer.load_or_create().unwrap();
        resolution.load_or_create().unwrap();
        resolution
            .rotate_retaining(&std::collections::BTreeSet::new())
            .unwrap();
        let again = geology.load_or_create().unwrap();
        assert_eq!(again.version(), 1);
        assert_eq!(again.fingerprint(), fingerprint);
        remove(&geology);
    }

    #[test]
    fn a_manifest_naming_a_retained_version_is_refused() {
        let store = temp_store("retained");
        store.load_or_create().unwrap();
        let key = std::fs::read(store.inner.key_path(1)).unwrap();
        std::fs::write(store.inner.key_path(2), &key).unwrap();
        std::fs::write(
            store.inner.manifest_path(2),
            br#"{"format_version":1,"current":2,"retained":[1]}"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for path in [store.inner.key_path(2), store.inner.manifest_path(2)] {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        assert!(
            store.load_or_create().is_err(),
            "a geology ring never carries a retained key"
        );
        remove(&store);
    }

    #[test]
    fn the_debug_form_carries_no_key_bytes() {
        let store = temp_store("debug");
        let secret = store.load_or_create().unwrap();
        let encoded = std::fs::read(store.inner.key_path(1)).unwrap();
        let encoded = std::str::from_utf8(&encoded).unwrap().trim().to_owned();
        assert!(!encoded.is_empty());
        let printed = format!("{secret:?} {store:?} {:?}", secret.mac());
        assert!(!printed.contains(&encoded));
        assert!(printed.contains("REDACTED"));
        remove(&store);
    }
}
