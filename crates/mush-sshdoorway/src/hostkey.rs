//! The doorway's host key (doc 29 §5): one ed25519 key at a fixed path,
//! loaded by every generation, so a restart never changes what players see in
//! `known_hosts`. Generated on first boot from the OS random source.

use std::path::Path;

use russh::keys::PrivateKey;
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::Ed25519Keypair;

/// Load the key at `path`, or generate and persist one.
pub fn load_or_generate(path: &Path) -> Result<PrivateKey, String> {
    let ctx = |e: &dyn std::fmt::Display| format!("host key {}: {e}", path.display());
    if path.exists() {
        return PrivateKey::read_openssh_file(path).map_err(|e| ctx(&e));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| ctx(&e))?;
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| ctx(&e))?;
    let key = PrivateKey::from(Ed25519Keypair::from_seed(&seed));
    key.write_openssh_file(path, LineEnding::LF)
        .map_err(|e| ctx(&e))?;
    tracing::warn!(path = %path.display(), "generated a new doorway host key");
    Ok(key)
}

/// The key's SHA-256 fingerprint, as OpenSSH prints it.
#[must_use]
pub fn fingerprint(key: &PrivateKey) -> String {
    key.public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_once_then_reloads_the_same_key() {
        let dir = std::env::temp_dir().join(format!(
            "sshdoorway-hostkey-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        let path = dir.join("host_ed25519");
        let a = load_or_generate(&path).unwrap();
        let b = load_or_generate(&path).unwrap();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert!(fingerprint(&a).starts_with("SHA256:"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
