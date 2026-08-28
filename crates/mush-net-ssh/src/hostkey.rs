//! Ed25519 host key: loaded from the data dir, generated on first boot.
//!
//! The key is persisted in OpenSSH PEM format and never regenerated (a new
//! key means a scary fingerprint change for every player); delete the file
//! deliberately to rotate.

use std::path::Path;

use russh::keys::ssh_key::rand_core::{TryCryptoRng, TryRng};
use russh::keys::ssh_key::{self, LineEnding};
use russh::keys::{Algorithm, PrivateKey};

use crate::SshError;

/// OS entropy as a `rand_core` 0.10 `CryptoRng` (ssh-key's generation API
/// wants one; the workspace's `rand 0.9` speaks an older `rand_core`).
/// OS-RNG failure is unrecoverable, so this treats it as fatal.
struct OsEntropy;

impl TryRng for OsEntropy {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(getrandom::u32().expect("OS random source unavailable"))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(getrandom::u64().expect("OS random source unavailable"))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        getrandom::fill(dst).expect("OS random source unavailable");
        Ok(())
    }
}

impl TryCryptoRng for OsEntropy {}

/// Load the host key at `path`, generating (and persisting) a fresh
/// ed25519 key if the file does not exist.
pub(crate) fn load_or_generate(path: &Path) -> Result<PrivateKey, SshError> {
    let context = |e: &dyn std::fmt::Display| SshError::HostKey {
        path: path.display().to_string(),
        detail: e.to_string(),
    };
    if path.exists() {
        let key = PrivateKey::read_openssh_file(path).map_err(|e| context(&e))?;
        tracing::info!(path = %path.display(), "loaded SSH host key");
        return Ok(key);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| context(&e))?;
    }
    let key = generate_ed25519().map_err(|e| context(&e))?;
    key.write_openssh_file(path, LineEnding::LF)
        .map_err(|e| context(&e))?;
    tracing::info!(path = %path.display(), "generated new ed25519 SSH host key");
    Ok(key)
}

fn generate_ed25519() -> Result<PrivateKey, ssh_key::Error> {
    PrivateKey::random(&mut OsEntropy, Algorithm::Ed25519)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_persist_reload_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "mush-net-ssh-hostkey-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        let path = dir.join("keys").join("host_ed25519");
        let first = load_or_generate(&path).expect("generate");
        assert!(path.exists());
        let second = load_or_generate(&path).expect("reload");
        assert_eq!(
            first.public_key().to_openssh().unwrap(),
            second.public_key().to_openssh().unwrap(),
            "reload must return the same key, not a new one"
        );
        assert_eq!(first.algorithm(), Algorithm::Ed25519);
        std::fs::remove_dir_all(&dir).ok();
    }
}
