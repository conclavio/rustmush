//! Shared crash-safe storage for independent versioned 256-bit key families.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const FORMAT_VERSION: u32 = 1;
pub(crate) const KEY_BYTES: usize = 32;
const ENCODED_KEY_BYTES: u64 = 65;
const MAX_MANIFEST_BYTES: u64 = 4 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 64;

pub(crate) struct VersionedKeyMaterial {
    pub(crate) version: u32,
    pub(crate) key: [u8; KEY_BYTES],
}

pub(crate) trait KeyFamily {
    type Ring;
    type RingError: std::error::Error + Send + Sync + 'static;
    const MAX_VERSIONS: usize;
    const KEY_PREFIX: &'static str;
    const MANIFEST_PREFIX: &'static str;

    fn build_ring(
        current: VersionedKeyMaterial,
        retained: Vec<VersionedKeyMaterial>,
    ) -> Result<Self::Ring, Self::RingError>;
}

/// Private append-only key directory shared by protocol-specific wrappers.
pub(crate) struct DurableKeyStore<F> {
    directory: PathBuf,
    family: PhantomData<fn() -> F>,
}

impl<F> Clone for DurableKeyStore<F> {
    fn clone(&self) -> Self {
        Self {
            directory: self.directory.clone(),
            family: PhantomData,
        }
    }
}

impl<F> std::fmt::Debug for DurableKeyStore<F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableKeyStore")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl<F: KeyFamily> DurableKeyStore<F> {
    pub(crate) fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            family: PhantomData,
        }
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn load_or_create(&self) -> Result<F::Ring, DurableKeyStoreError<F::RingError>> {
        self.prepare_directory()?;
        match self.latest_manifest()? {
            Some((version, path)) => self.load_manifest(version, &path),
            None => self.provision_initial(),
        }
    }

    pub(crate) fn rotate(&self) -> Result<F::Ring, DurableKeyStoreError<F::RingError>> {
        self.prepare_directory()?;
        let Some((current_version, current_path)) = self.latest_manifest()? else {
            return self.provision_initial();
        };
        let manifest = self.read_manifest(current_version, &current_path)?;
        let _ = self.ring_from_manifest(&manifest)?;
        let next = current_version
            .checked_add(1)
            .ok_or(DurableKeyStoreError::VersionExhausted)?;
        self.create_or_recover_key(next)?;

        let mut retained = Vec::with_capacity(F::MAX_VERSIONS.saturating_sub(1));
        retained.push(manifest.current);
        retained.extend(manifest.retained.iter().copied());
        retained.truncate(F::MAX_VERSIONS.saturating_sub(1));
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

    fn prepare_directory(&self) -> Result<(), DurableKeyStoreError<F::RingError>> {
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
                    .map_err(|source| DurableKeyStoreError::Io {
                        path: self.directory.clone(),
                        source,
                    })?;
            }
            Err(source) => {
                return Err(DurableKeyStoreError::Io {
                    path: self.directory.clone(),
                    source,
                });
            }
        }
        let metadata = std::fs::symlink_metadata(&self.directory).map_err(|source| {
            DurableKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            }
        })?;
        if !metadata.file_type().is_dir() {
            return Err(DurableKeyStoreError::UnsafePath {
                path: self.directory.clone(),
                detail: "key store is not a real directory",
            });
        }
        enforce_private_directory(&self.directory, &metadata)?;
        Ok(())
    }

    fn latest_manifest(
        &self,
    ) -> Result<Option<(u32, PathBuf)>, DurableKeyStoreError<F::RingError>> {
        let entries =
            std::fs::read_dir(&self.directory).map_err(|source| DurableKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
        let mut count = 0_usize;
        let mut latest = None;
        for entry in entries {
            count = count
                .checked_add(1)
                .ok_or(DurableKeyStoreError::DirectoryEntryLimit {
                    max: MAX_DIRECTORY_ENTRIES,
                })?;
            if count > MAX_DIRECTORY_ENTRIES {
                return Err(DurableKeyStoreError::DirectoryEntryLimit {
                    max: MAX_DIRECTORY_ENTRIES,
                });
            }
            let entry = entry.map_err(|source| DurableKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(version) = parse_versioned_name(&name, F::MANIFEST_PREFIX, ".json") else {
                continue;
            };
            if latest.as_ref().is_none_or(|(seen, _)| version > *seen) {
                latest = Some((version, entry.path()));
            }
        }
        Ok(latest)
    }

    fn provision_initial(&self) -> Result<F::Ring, DurableKeyStoreError<F::RingError>> {
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
    ) -> Result<F::Ring, DurableKeyStoreError<F::RingError>> {
        let manifest = self.read_manifest(filename_version, path)?;
        self.ring_from_manifest(&manifest)
    }

    fn read_manifest(
        &self,
        filename_version: u32,
        path: &Path,
    ) -> Result<KeyRingManifest, DurableKeyStoreError<F::RingError>> {
        let bytes = read_private_bounded_file(path, MAX_MANIFEST_BYTES)?;
        let manifest: KeyRingManifest =
            serde_json::from_slice(&bytes).map_err(|source| DurableKeyStoreError::Manifest {
                path: path.to_owned(),
                detail: source.to_string(),
            })?;
        manifest.validate::<F>(filename_version, path)?;
        Ok(manifest)
    }

    fn ring_from_manifest(
        &self,
        manifest: &KeyRingManifest,
    ) -> Result<F::Ring, DurableKeyStoreError<F::RingError>> {
        let current = VersionedKeyMaterial {
            version: manifest.current,
            key: self.read_key(manifest.current)?,
        };
        let retained = manifest
            .retained
            .iter()
            .map(|version| {
                Ok(VersionedKeyMaterial {
                    version: *version,
                    key: self.read_key(*version)?,
                })
            })
            .collect::<Result<Vec<_>, DurableKeyStoreError<F::RingError>>>()?;
        F::build_ring(current, retained).map_err(DurableKeyStoreError::Ring)
    }

    fn read_key(
        &self,
        version: u32,
    ) -> Result<[u8; KEY_BYTES], DurableKeyStoreError<F::RingError>> {
        let path = self.key_path(version);
        let encoded = read_private_bounded_file(&path, ENCODED_KEY_BYTES)?;
        decode_key(&encoded).map_err(|detail| DurableKeyStoreError::Key { path, detail })
    }

    pub(crate) fn create_or_recover_key(
        &self,
        version: u32,
    ) -> Result<(), DurableKeyStoreError<F::RingError>> {
        let path = self.key_path(version);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
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
                    .map_err(|source| DurableKeyStoreError::Io {
                        path: path.clone(),
                        source,
                    })?;
                fsync_dir(&self.directory).map_err(|source| DurableKeyStoreError::Io {
                    path: self.directory.clone(),
                    source,
                })
            }
            Err(source) => Err(DurableKeyStoreError::Io { path, source }),
        }
    }

    fn publish_manifest(
        &self,
        manifest: &KeyRingManifest,
    ) -> Result<(), DurableKeyStoreError<F::RingError>> {
        let path = self.manifest_path(manifest.current);
        let bytes =
            serde_json::to_vec(manifest).map_err(|source| DurableKeyStoreError::Manifest {
                path: path.clone(),
                detail: source.to_string(),
            })?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_MANIFEST_BYTES {
            return Err(DurableKeyStoreError::Manifest {
                path,
                detail: "encoded manifest exceeds its hard byte limit".to_owned(),
            });
        }
        match secure_create_new(&path) {
            Ok(mut file) => {
                file.write_all(&bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|source| DurableKeyStoreError::Io {
                        path: path.clone(),
                        source,
                    })?;
                fsync_dir(&self.directory).map_err(|source| DurableKeyStoreError::Io {
                    path: self.directory.clone(),
                    source,
                })
            }
            Err(DurableKeyStoreError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                let existing = self.read_manifest(manifest.current, &path)?;
                if &existing == manifest {
                    Ok(())
                } else {
                    Err(DurableKeyStoreError::Manifest {
                        path,
                        detail: "existing immutable manifest differs".to_owned(),
                    })
                }
            }
            Err(error) => Err(error),
        }
    }

    fn prune_after_publish(
        &self,
        manifest: &KeyRingManifest,
    ) -> Result<(), DurableKeyStoreError<F::RingError>> {
        let retained: BTreeSet<_> = std::iter::once(manifest.current)
            .chain(manifest.retained.iter().copied())
            .collect();
        let entries =
            std::fs::read_dir(&self.directory).map_err(|source| DurableKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
        let mut removed = false;
        for entry in entries {
            let entry = entry.map_err(|source| DurableKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let key_version = parse_versioned_name(&name, F::KEY_PREFIX, ".hex");
            let manifest_version = parse_versioned_name(&name, F::MANIFEST_PREFIX, ".json");
            let should_remove = key_version.is_some_and(|version| !retained.contains(&version))
                || manifest_version.is_some_and(|version| version != manifest.current);
            if should_remove {
                std::fs::remove_file(entry.path()).map_err(|source| DurableKeyStoreError::Io {
                    path: entry.path(),
                    source,
                })?;
                removed = true;
            }
        }
        if removed {
            fsync_dir(&self.directory).map_err(|source| DurableKeyStoreError::Io {
                path: self.directory.clone(),
                source,
            })?;
        }
        Ok(())
    }

    pub(crate) fn key_path(&self, version: u32) -> PathBuf {
        self.directory
            .join(format!("{}{version:010}.hex", F::KEY_PREFIX))
    }

    pub(crate) fn manifest_path(&self, version: u32) -> PathBuf {
        self.directory
            .join(format!("{}{version:010}.json", F::MANIFEST_PREFIX))
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
    fn validate<F: KeyFamily>(
        &self,
        filename_version: u32,
        path: &Path,
    ) -> Result<(), DurableKeyStoreError<F::RingError>> {
        if self.format_version != FORMAT_VERSION {
            return Err(manifest_error(path, "unsupported format version"));
        }
        if self.current == 0 || self.current != filename_version {
            return Err(manifest_error(
                path,
                "current version does not match filename",
            ));
        }
        if self.retained.len() >= F::MAX_VERSIONS {
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

fn manifest_error<E>(path: &Path, detail: &'static str) -> DurableKeyStoreError<E> {
    DurableKeyStoreError::Manifest {
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

fn read_private_bounded_file<E>(
    path: &Path,
    maximum: u64,
) -> Result<Vec<u8>, DurableKeyStoreError<E>> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| DurableKeyStoreError::Io {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(DurableKeyStoreError::UnsafePath {
            path: path.to_owned(),
            detail: "key material is not a regular file",
        });
    }
    enforce_private_file(path, &metadata)?;
    if metadata.len() > maximum {
        return Err(DurableKeyStoreError::FileTooLarge {
            path: path.to_owned(),
            actual: metadata.len(),
            max: maximum,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)
        .and_then(|file| file.take(maximum.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|source| DurableKeyStoreError::Io {
            path: path.to_owned(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum {
        return Err(DurableKeyStoreError::FileTooLarge {
            path: path.to_owned(),
            actual: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            max: maximum,
        });
    }
    Ok(bytes)
}

fn secure_create_new<E>(path: &Path) -> Result<File, DurableKeyStoreError<E>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| DurableKeyStoreError::Io {
            path: path.to_owned(),
            source,
        })
}

#[cfg(unix)]
fn enforce_private_directory<E>(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), DurableKeyStoreError<E>> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(DurableKeyStoreError::UnsafePermissions {
            path: path.to_owned(),
            required: "0700",
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_private_directory<E>(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), DurableKeyStoreError<E>> {
    Ok(())
}

#[cfg(unix)]
fn enforce_private_file<E>(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), DurableKeyStoreError<E>> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(DurableKeyStoreError::UnsafePermissions {
            path: path.to_owned(),
            required: "0600",
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_private_file<E>(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), DurableKeyStoreError<E>> {
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

/// Fail-closed durable private-key error. No variant carries secret bytes.
#[derive(Debug, Error)]
pub enum DurableKeyStoreError<E> {
    #[error("durable key I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unsafe durable key path {path}: {detail}")]
    UnsafePath { path: PathBuf, detail: &'static str },
    #[error("durable key path {path} must have private mode {required}")]
    UnsafePermissions {
        path: PathBuf,
        required: &'static str,
    },
    #[error("durable key file {path} is {actual} bytes; maximum is {max}")]
    FileTooLarge {
        path: PathBuf,
        actual: u64,
        max: u64,
    },
    #[error("invalid durable key file {path}: {detail}")]
    Key { path: PathBuf, detail: &'static str },
    #[error("invalid durable key manifest {path}: {detail}")]
    Manifest { path: PathBuf, detail: String },
    #[error("durable key directory exceeds its hard limit of {max} entries")]
    DirectoryEntryLimit { max: usize },
    #[error("durable key version space is exhausted")]
    VersionExhausted,
    #[error("invalid typed key ring: {0}")]
    Ring(E),
}

#[cfg(test)]
mod tests {
    use super::*;

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
