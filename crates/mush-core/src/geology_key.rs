//! The geology secret — the key behind the ground (doc 20 cut SM.1; §3.2, D-20-4, Q-20-12).
//!
//! Surface mining's geology is a pure keyed function of a body-fixed position: HMAC-SHA256 under
//! this secret over the cell's address (`tacsim::surface::geology`). The key is a real deployment
//! secret, the fourth durable key family beside the offer, resolution and decision rings
//! (`mush_config::geology_keys`), with three differences doc 20 rules:
//!
//! - **one version, never rotated** ([`MAX_GEOLOGY_SECRET_VERSIONS`] is 1): rotating it would
//!   move every reserve and every pocket on every body, so a ring of one is the whole family;
//! - **a lost key is a disabled layer, not a corrupted one** (§16): the host hands a key that
//!   would not load to the game crate as a disabled state and never refuses the boot;
//! - **the world pins its fingerprint** ([`GeologySecret::fingerprint`], SHA-256 of the 32 key
//!   bytes) at the first keyed boot, so a restore with the wrong key directory reads disabled
//!   instead of quietly re-rolling the ground under every claim.
//!
//! Neither type here is serializable or printable past its version: the key bytes and the keyed
//! HMAC state never leave the process (`truth_serialization_policy.rs` asserts it).

use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The geology family keeps exactly one version (D-20-4, Q-20-12: never rotated).
pub const MAX_GEOLOGY_SECRET_VERSIONS: usize = 1;

/// The one geology key a deployment holds. The HMAC state is keyed once here, in [`Self::new`],
/// and cloned per draw by [`GeologyMac::draw`], so a draw never re-keys and never fails.
#[derive(Clone)]
pub struct GeologySecret {
    version: u32,
    key: [u8; 32],
    mac: Hmac<Sha256>,
}

impl fmt::Debug for GeologySecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeologySecret")
            .field("version", &self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl GeologySecret {
    /// The secret for `version` over `key`. Refuses version 0 and the all-zero key, as the other
    /// three families do; keying the HMAC cannot fail for 32 bytes, and if it ever did the
    /// refusal is [`GeologySecretError::Key`], never a panic.
    pub fn new(version: u32, key: [u8; 32]) -> Result<Self, GeologySecretError> {
        if version == 0 {
            return Err(GeologySecretError::ZeroVersion);
        }
        if key == [0; 32] {
            return Err(GeologySecretError::ZeroKey);
        }
        let mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| GeologySecretError::Key)?;
        Ok(Self { version, key, mac })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// SHA-256 of the 32 key bytes: what the world pins at its first keyed boot (doc 20 SM.1,
    /// Q-20-12) and compares at every later one. It names the key without revealing it.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; 32] {
        Sha256::digest(self.key).into()
    }

    /// A pre-keyed HMAC-SHA256 state for the geology draws, cloned per draw.
    #[must_use]
    pub fn mac(&self) -> GeologyMac {
        GeologyMac(self.mac.clone())
    }
}

/// The keyed HMAC-SHA256 state the geology function draws with. It holds no way back to the key
/// bytes a caller could print or serialize.
#[derive(Clone)]
pub struct GeologyMac(Hmac<Sha256>);

impl fmt::Debug for GeologyMac {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GeologyMac([REDACTED])")
    }
}

impl GeologyMac {
    /// HMAC-SHA256(key, message): clone the keyed state, update, finalize. Pure: the same key and
    /// message give the same 32 bytes forever.
    #[must_use]
    pub fn draw(&self, message: &[u8]) -> [u8; 32] {
        let mut mac = self.0.clone();
        mac.update(message);
        mac.finalize().into_bytes().into()
    }
}

/// Why a geology secret was refused. No variant carries key bytes.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GeologySecretError {
    #[error("geology secret version zero is reserved")]
    ZeroVersion,
    #[error("an all-zero geology secret is not accepted")]
    ZeroKey,
    #[error("the geology key could not key HMAC-SHA256")]
    Key,
    #[error("the geology key family keeps one version and never retains another")]
    Retained,
}

/// What the boot path hands the surface geology installer from the operator's
/// `[tacsim].geology_key_file` setting.
///
/// The boot continues under every variant (a world with no surface mining never
/// needs the key); the installer converts this into the live geology slot's
/// state and logs the refuse reason when one applies.
#[derive(Debug, Clone)]
pub enum GeologyBoot {
    /// No `geology_key_file` was set in `mush.toml`.
    NotConfigured,
    /// The file was read and validated into a [`GeologySecret`].
    Key(GeologySecret),
    /// A path was configured, and reading or validating it failed. The string carries no key
    /// material (`KeySourceError` never embeds any).
    Unreadable(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = *b"rustmush-sm1-geology-test-key-01";

    fn base16(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn new_refuses_version_zero_and_the_all_zero_key() {
        assert_eq!(
            GeologySecret::new(0, KEY).err(),
            Some(GeologySecretError::ZeroVersion)
        );
        assert_eq!(
            GeologySecret::new(1, [0; 32]).err(),
            Some(GeologySecretError::ZeroKey)
        );
        assert!(GeologySecret::new(1, KEY).is_ok());
    }

    #[test]
    fn the_fingerprint_is_sha256_of_the_key_bytes() {
        let secret = GeologySecret::new(1, KEY).unwrap();
        let want: [u8; 32] = Sha256::digest(KEY).into();
        assert_eq!(secret.fingerprint(), want);
        assert_eq!(secret.version(), 1);
    }

    #[test]
    fn a_draw_is_the_plain_hmac_of_the_message_every_time() {
        let secret = GeologySecret::new(1, KEY).unwrap();
        let mac = secret.mac();
        for message in [&b""[..], b"rustmush.geology.v1\0ceres", b"\x00\x01\x02"] {
            let mut direct = Hmac::<Sha256>::new_from_slice(&KEY).unwrap();
            direct.update(message);
            let want: [u8; 32] = direct.finalize().into_bytes().into();
            assert_eq!(mac.draw(message), want);
            assert_eq!(mac.draw(message), want, "a second draw is the same draw");
        }
    }

    #[test]
    fn the_debug_forms_carry_no_key_bytes() {
        let secret = GeologySecret::new(7, KEY).unwrap();
        let printed = format!("{secret:?} {:?}", secret.mac());
        assert!(printed.contains("REDACTED"));
        assert!(printed.contains('7'));
        assert!(!printed.contains(&base16(&KEY)));
        assert!(!printed.contains("rustmush-sm1-geology-test-key-01"));
    }
}
