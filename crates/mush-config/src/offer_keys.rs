//! Durable key provisioning for opaque K5 legal-action offers.
//!
//! Keys and manifests are immutable files created with `create_new`. Rotation
//! writes and fsyncs the next key before publishing the next manifest, so a
//! crash cannot replace the last readable ring. A highest manifest that exists
//! but is malformed fails boot closed rather than silently selecting older key
//! material and invalidating in-flight offers.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use mush_core::{MAX_OFFER_SECRET_VERSIONS, OfferKeyRing, OfferSecret, OfferSecretError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const FORMAT_VERSION: u32 = 1;
const KEY_BYTES: usize = 32;
const ENCODED_KEY_BYTES: u64 = 65;
const MAX_MANIFEST_BYTES: u64 = 4 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 64;

/// A private, append-only deployment key directory.
#[derive(Clone, Debug)]
pub struct DurableOfferKeyStore {
    directory: PathBuf,
}

impl DurableOfferKeyStore {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Load the newest complete ring or provision version one. The caller must
    /// hold the deployment's single-server lifetime lock.
    pub fn load_or_create(&self) -> Result<OfferKeyRing, OfferKeyStoreError> {
        self.prepare_directory()?;
        match self.latest_manifest()? {
            Some((version, path)) => self.load_manifest(version, &path),
            None => self.provision_initial(),
        }
    }

    /// Durably publish a new current key while retaining the newest bounded
    /// verification history. The old manifest remains authoritative until the
    /// new manifest has been fully written and fsynced.
    pub fn rotate(&self) -> Result<OfferKeyRing, OfferKeyStoreError> {
        self.prepare_directory()?;
        let Some((current_version, current_path)) = self.latest_manifest()? else {
            return self.provision_initial();
        };
        let manifest = self.read_manifest(current_version, &current_path)?;
        // Validate every referenced key before deriving a successor.
        let _ = self.ring_from_manifest(&manifest)?;
        let next = current_version
            .checked_add(1)
            .ok_or(OfferKeyStoreError::VersionExhausted)?;
        self.create_or_recover_key(next)?;

        let mut retained = Vec::with_capacity(MAX_OFFER_SECRET_VERSIONS.saturating_sub(1));
        retained.push(manifest.current);
        retained.extend(manifest.retained.iter().copied());
        retained.truncate(MAX_OFFER_SECRET_VERSIONS.saturating_sub(1));
        let successor = KeyRingManifest {
            format_version: FORMAT_VERSION,
            current: next,
            retained,
        };
        self.publish_manifest(&successor)?;
        let ring = self.ring_from_manifest(&successor)?;
        self.prune_after_publish(&successor)?;
        Ok(ring)
    }

    fn prepare_directory(&self) -> Result<(), OfferKeyStoreError> {
        match std::fs::symlink_metadata(&self.directory) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt as _;
                    builder.mode(0o700);
                }
                builder
                    .create(&self.directory)
                    .map_err(|source| OfferKeyStoreError::Io {
                        path: self.directory.clone(),
                        source,
                    })?;
            }
            Err(source) => {
                return Err(OfferKeyStoreError::Io {
                    path: self.directory.clone(),
                    source,
                });
            }
        }
        let metadata = std::fs::symlink_metadata(&self.directory).map_err(|source| {
            OfferKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            }
        })?;
        if !metadata.file_type().is_dir() {
            return Err(OfferKeyStoreError::UnsafePath {
                path: self.directory.clone(),
                detail: "key store is not a real directory",
            });
        }
        enforce_private_directory(&self.directory, &metadata)?;
        Ok(())
    }

    fn latest_manifest(&self) -> Result<Option<(u32, PathBuf)>, OfferKeyStoreError> {
        let entries =
            std::fs::read_dir(&self.directory).map_err(|source| OfferKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
        let mut count = 0_usize;
        let mut latest = None;
        for entry in entries {
            count = count
                .checked_add(1)
                .ok_or(OfferKeyStoreError::DirectoryEntryLimit {
                    max: MAX_DIRECTORY_ENTRIES,
                })?;
            if count > MAX_DIRECTORY_ENTRIES {
                return Err(OfferKeyStoreError::DirectoryEntryLimit {
                    max: MAX_DIRECTORY_ENTRIES,
                });
            }
            let entry = entry.map_err(|source| OfferKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(version) = parse_versioned_name(&name, "ring-", ".json") else {
                continue;
            };
            if latest.as_ref().is_none_or(|(seen, _)| version > *seen) {
                latest = Some((version, entry.path()));
            }
        }
        Ok(latest)
    }

    fn provision_initial(&self) -> Result<OfferKeyRing, OfferKeyStoreError> {
        self.create_or_recover_key(1)?;
        let manifest = KeyRingManifest {
            format_version: FORMAT_VERSION,
            current: 1,
            retained: Vec::new(),
        };
        self.publish_manifest(&manifest)?;
        self.ring_from_manifest(&manifest)
    }

    fn load_manifest(
        &self,
        filename_version: u32,
        path: &Path,
    ) -> Result<OfferKeyRing, OfferKeyStoreError> {
        let manifest = self.read_manifest(filename_version, path)?;
        self.ring_from_manifest(&manifest)
    }

    fn read_manifest(
        &self,
        filename_version: u32,
        path: &Path,
    ) -> Result<KeyRingManifest, OfferKeyStoreError> {
        let bytes = read_private_bounded_file(path, MAX_MANIFEST_BYTES)?;
        let manifest: KeyRingManifest =
            serde_json::from_slice(&bytes).map_err(|source| OfferKeyStoreError::Manifest {
                path: path.to_owned(),
                detail: source.to_string(),
            })?;
        manifest.validate(filename_version, path)?;
        Ok(manifest)
    }

    fn ring_from_manifest(
        &self,
        manifest: &KeyRingManifest,
    ) -> Result<OfferKeyRing, OfferKeyStoreError> {
        let current = OfferSecret::new(manifest.current, self.read_key(manifest.current)?)?;
        let retained = manifest
            .retained
            .iter()
            .map(|version| Ok(OfferSecret::new(*version, self.read_key(*version)?)?))
            .collect::<Result<Vec<_>, OfferKeyStoreError>>()?;
        OfferKeyRing::new(current, retained).map_err(OfferKeyStoreError::from)
    }

    fn read_key(&self, version: u32) -> Result<[u8; KEY_BYTES], OfferKeyStoreError> {
        let path = self.key_path(version);
        let encoded = read_private_bounded_file(&path, ENCODED_KEY_BYTES)?;
        decode_key(&encoded).map_err(|detail| OfferKeyStoreError::Key { path, detail })
    }

    fn create_or_recover_key(&self, version: u32) -> Result<(), OfferKeyStoreError> {
        let path = self.key_path(version);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                // A key can legitimately precede its manifest after a crash.
                let _ = self.read_key(version)?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut key: [u8; KEY_BYTES] = rand::random();
                while key == [0; KEY_BYTES] {
                    key = rand::random();
                }
                let encoded = encode_key(key);
                let mut file = secure_create_new(&path)?;
                file.write_all(encoded.as_bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|source| OfferKeyStoreError::Io {
                        path: path.clone(),
                        source,
                    })?;
                fsync_dir(&self.directory).map_err(|source| OfferKeyStoreError::Io {
                    path: self.directory.clone(),
                    source,
                })
            }
            Err(source) => Err(OfferKeyStoreError::Io { path, source }),
        }
    }

    fn publish_manifest(&self, manifest: &KeyRingManifest) -> Result<(), OfferKeyStoreError> {
        let path = self.manifest_path(manifest.current);
        let bytes =
            serde_json::to_vec(manifest).map_err(|source| OfferKeyStoreError::Manifest {
                path: path.clone(),
                detail: source.to_string(),
            })?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_MANIFEST_BYTES {
            return Err(OfferKeyStoreError::Manifest {
                path,
                detail: "encoded manifest exceeds its hard byte limit".to_owned(),
            });
        }
        match secure_create_new(&path) {
            Ok(mut file) => {
                file.write_all(&bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|source| OfferKeyStoreError::Io {
                        path: path.clone(),
                        source,
                    })?;
                fsync_dir(&self.directory).map_err(|source| OfferKeyStoreError::Io {
                    path: self.directory.clone(),
                    source,
                })
            }
            Err(OfferKeyStoreError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                let existing = self.read_manifest(manifest.current, &path)?;
                if &existing == manifest {
                    Ok(())
                } else {
                    Err(OfferKeyStoreError::Manifest {
                        path,
                        detail: "existing immutable manifest differs".to_owned(),
                    })
                }
            }
            Err(error) => Err(error),
        }
    }

    fn prune_after_publish(&self, manifest: &KeyRingManifest) -> Result<(), OfferKeyStoreError> {
        let retained: BTreeSet<_> = std::iter::once(manifest.current)
            .chain(manifest.retained.iter().copied())
            .collect();
        let entries =
            std::fs::read_dir(&self.directory).map_err(|source| OfferKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
        let mut removed = false;
        for entry in entries {
            let entry = entry.map_err(|source| OfferKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let key_version = parse_versioned_name(&name, "key-", ".hex");
            let manifest_version = parse_versioned_name(&name, "ring-", ".json");
            let should_remove = key_version.is_some_and(|version| !retained.contains(&version))
                || manifest_version.is_some_and(|version| version != manifest.current);
            if should_remove {
                std::fs::remove_file(entry.path()).map_err(|source| OfferKeyStoreError::Io {
                    path: entry.path(),
                    source,
                })?;
                removed = true;
            }
        }
        if removed {
            fsync_dir(&self.directory).map_err(|source| OfferKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
        }
        Ok(())
    }

    fn key_path(&self, version: u32) -> PathBuf {
        self.directory.join(format!("key-{version:010}.hex"))
    }

    fn manifest_path(&self, version: u32) -> PathBuf {
        self.directory.join(format!("ring-{version:010}.json"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct KeyRingManifest {
    format_version: u32,
    current: u32,
    retained: Vec<u32>,
}

impl KeyRingManifest {
    fn validate(&self, filename_version: u32, path: &Path) -> Result<(), OfferKeyStoreError> {
        if self.format_version != FORMAT_VERSION {
            return Err(manifest_error(path, "unsupported format version"));
        }
        if self.current == 0 || self.current != filename_version {
            return Err(manifest_error(
                path,
                "current version does not match filename",
            ));
        }
        if self.retained.len() >= MAX_OFFER_SECRET_VERSIONS {
            return Err(manifest_error(
                path,
                "retained key count exceeds hard limit",
            ));
        }
        let unique: BTreeSet<_> = self.retained.iter().copied().collect();
        if unique.len() != self.retained.len()
            || self
                .retained
                .iter()
                .any(|version| *version == 0 || *version >= self.current)
        {
            return Err(manifest_error(path, "retained versions are invalid"));
        }
        if !self.retained.windows(2).all(|pair| pair[0] > pair[1]) {
            return Err(manifest_error(
                path,
                "retained versions are not newest-first",
            ));
        }
        Ok(())
    }
}

fn manifest_error(path: &Path, detail: &'static str) -> OfferKeyStoreError {
    OfferKeyStoreError::Manifest {
        path: path.to_owned(),
        detail: detail.to_owned(),
    }
}

fn parse_versioned_name(name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    let digits = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if digits.len() != 10 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|version| *version != 0)
}

fn encode_key(key: [u8; KEY_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(KEY_BYTES * 2 + 1);
    for byte in key {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded.push('\n');
    encoded
}

fn decode_key(encoded: &[u8]) -> Result<[u8; KEY_BYTES], &'static str> {
    if encoded.len() != KEY_BYTES * 2 + 1 || encoded.last() != Some(&b'\n') {
        return Err("private key file has the wrong length");
    }
    let mut key = [0_u8; KEY_BYTES];
    for (index, slot) in key.iter_mut().enumerate() {
        let high = decode_nibble(encoded[index * 2]).ok_or("private key is not lowercase hex")?;
        let low =
            decode_nibble(encoded[index * 2 + 1]).ok_or("private key is not lowercase hex")?;
        *slot = (high << 4) | low;
    }
    if key == [0; KEY_BYTES] {
        return Err("private key is all zero");
    }
    Ok(key)
}

const fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn read_private_bounded_file(path: &Path, maximum: u64) -> Result<Vec<u8>, OfferKeyStoreError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| OfferKeyStoreError::Io {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(OfferKeyStoreError::UnsafePath {
            path: path.to_owned(),
            detail: "key material is not a regular file",
        });
    }
    enforce_private_file(path, &metadata)?;
    if metadata.len() > maximum {
        return Err(OfferKeyStoreError::FileTooLarge {
            path: path.to_owned(),
            actual: metadata.len(),
            max: maximum,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)
        .and_then(|file| file.take(maximum.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|source| OfferKeyStoreError::Io {
            path: path.to_owned(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum {
        return Err(OfferKeyStoreError::FileTooLarge {
            path: path.to_owned(),
            actual: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            max: maximum,
        });
    }
    Ok(bytes)
}

fn secure_create_new(path: &Path) -> Result<File, OfferKeyStoreError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path).map_err(|source| OfferKeyStoreError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(unix)]
fn enforce_private_directory(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), OfferKeyStoreError> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(OfferKeyStoreError::UnsafePermissions {
            path: path.to_owned(),
            required: "0700",
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_private_directory(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), OfferKeyStoreError> {
    Ok(())
}

#[cfg(unix)]
fn enforce_private_file(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), OfferKeyStoreError> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(OfferKeyStoreError::UnsafePermissions {
            path: path.to_owned(),
            required: "0600",
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_private_file(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), OfferKeyStoreError> {
    Ok(())
}

#[cfg(unix)]
fn fsync_dir(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn fsync_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Fail-closed durable offer-key error. No variant carries secret bytes.
#[derive(Debug, Error)]
pub enum OfferKeyStoreError {
    #[error("offer key I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unsafe offer key path {path}: {detail}")]
    UnsafePath { path: PathBuf, detail: &'static str },
    #[error("offer key path {path} must have private mode {required}")]
    UnsafePermissions {
        path: PathBuf,
        required: &'static str,
    },
    #[error("offer key file {path} is {actual} bytes; maximum is {max}")]
    FileTooLarge {
        path: PathBuf,
        actual: u64,
        max: u64,
    },
    #[error("invalid offer key file {path}: {detail}")]
    Key { path: PathBuf, detail: &'static str },
    #[error("invalid offer key manifest {path}: {detail}")]
    Manifest { path: PathBuf, detail: String },
    #[error("offer key directory exceeds its hard limit of {max} entries")]
    DirectoryEntryLimit { max: usize },
    #[error("offer key version space is exhausted")]
    VersionExhausted,
    #[error("invalid offer key ring: {0}")]
    Ring(#[from] OfferSecretError),
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
        let key_path = store.key_path(1);
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
        assert!(!store.key_path(1).exists());
        for retained in 2..=5 {
            assert!(store.key_path(retained).exists());
        }
        assert_eq!(store.load_or_create().unwrap().current_version(), 5);
        remove(&store);
    }

    #[test]
    fn orphan_successor_key_is_reused_after_pre_manifest_crash() {
        let store = temp_store("orphan");
        store.load_or_create().unwrap();
        store.create_or_recover_key(2).unwrap();
        let orphan = std::fs::read(store.key_path(2)).unwrap();
        assert_eq!(store.rotate().unwrap().current_version(), 2);
        assert_eq!(std::fs::read(store.key_path(2)).unwrap(), orphan);
        remove(&store);
    }

    #[test]
    fn corrupt_highest_manifest_fails_closed() {
        let store = temp_store("corrupt");
        store.load_or_create().unwrap();
        let path = store.manifest_path(2);
        let mut file = secure_create_new(&path).unwrap();
        file.write_all(b"{").unwrap();
        file.sync_all().unwrap();
        assert!(matches!(
            store.load_or_create(),
            Err(OfferKeyStoreError::Manifest { .. })
        ));
        remove(&store);
    }

    #[test]
    fn versioned_names_are_strict() {
        assert_eq!(
            parse_versioned_name("ring-0000000007.json", "ring-", ".json"),
            Some(7)
        );
        assert_eq!(parse_versioned_name("ring-7.json", "ring-", ".json"), None);
        assert_eq!(
            parse_versioned_name("ring-0000000000.json", "ring-", ".json"),
            None
        );
    }
}
