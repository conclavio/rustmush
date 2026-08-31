//! Store-backed account helpers shared by the gateway (auth) and the server
//! (boot import, `@newpassword` hashing): validation, argon2id hashing /
//! verification, and the one-time `accounts.json` import.
//!
//! The pluggable [`AuthBackend`](../../mush_gateway/accounts) seam and its
//! production `Accounts` implementation live in `mush-gateway` (they need the
//! gateway's `GatewayLink`); everything here is transport-agnostic and pulls no
//! net / world code, so both halves can share it.

use std::path::Path;
use std::sync::OnceLock;

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use mush_core::ObjId;
use mush_store::{AccountRec, CharacterRef, ObjectStore, StoreError};
use serde::{Deserialize, Serialize};

use crate::unix_now_s;

/// Account names no one may register (routing usernames, staff identities).
pub const RESERVED_NAMES: &[&str] = &["god", "play", "guest", "new", "root", "admin"];

/// One character on an account. Mirrors the store's [`CharacterRef`] with
/// the core [`ObjId`] type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharSummary {
    /// The character's object id.
    pub id: ObjId,
    /// The character's name when it was linked.
    pub name: String,
}

impl From<CharacterRef> for CharSummary {
    fn from(c: CharacterRef) -> CharSummary {
        CharSummary {
            id: c.id,
            name: c.name,
        }
    }
}

/// Account/character-creation failure (player-facing messages).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CreateErr {
    /// The account name breaks the rules.
    #[error("account names are 3-20 letters, digits, '_' or '-'")]
    BadAccountName,
    /// The account name is reserved.
    #[error("that account name is reserved")]
    ReservedName,
    /// An account with that name already exists.
    #[error("an account with that name already exists")]
    AccountExists,
    /// `newchar` against an account that does not exist (should not happen
    /// through the normal flow — the player just authenticated).
    #[error("no such account")]
    NoSuchAccount,
    /// The account is at its character cap.
    #[error("that account already has the maximum number of characters")]
    CharacterLimit,
    /// The password breaks the rules.
    #[error("passwords must be 4-128 characters")]
    BadPassword,
    /// The character name breaks the rules.
    #[error("character names start with a letter and are 2-20 letters, digits, '_' or '-'")]
    BadCharacterName,
    /// A character with that name already exists in the world.
    #[error("a character with that name already exists")]
    CharacterNameTaken,
    /// The world task refused or is gone.
    #[error("character creation failed: {0}")]
    World(String),
    /// The persistence layer failed.
    #[error("account storage failed: {0}")]
    Io(String),
}

// ---------------------------------------------------------------------------
// One-time accounts.json import
// ---------------------------------------------------------------------------

/// The M1 interim file format, kept only so old files can be imported.
#[derive(Deserialize)]
struct LegacyAccountsFile {
    #[allow(dead_code)]
    version: u32,
    accounts: Vec<LegacyAccountRecord>,
}

#[derive(Deserialize)]
struct LegacyAccountRecord {
    name: String,
    hash: String,
    characters: Vec<CharSummary>,
}

/// If `data_dir/accounts.json` exists, import every record into the store
/// (an account already in the store wins and is skipped with a log line),
/// then rename the file to `accounts.json.imported`. Returns how many
/// accounts were imported; `Ok(0)` when there was no file.
///
/// A corrupt file is a boot error — silently skipping real account data
/// would strand every player in it.
///
/// # Errors
///
/// Read/parse/store/rename failures during the import.
pub async fn import_accounts_json(
    store: &dyn ObjectStore,
    data_dir: &Path,
) -> Result<usize, String> {
    let path = data_dir.join("accounts.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let file: LegacyAccountsFile =
        serde_json::from_str(&text).map_err(|e| format!("{} is corrupt: {e}", path.display()))?;

    let mut imported = 0usize;
    let now = unix_now_s();
    for rec in file.accounts {
        let name = rec.name.clone();
        let account = AccountRec {
            name: rec.name,
            argon2_hash: rec.hash,
            created: now,
            characters: rec
                .characters
                .into_iter()
                .map(|c| CharacterRef {
                    id: c.id,
                    name: c.name,
                })
                .collect(),
            ssh_keys: Vec::new(),
        };
        match store.create_account(account).await {
            Ok(()) => imported += 1,
            Err(StoreError::AccountExists) => {
                tracing::warn!(account = name, "import: account already in store; skipped");
            }
            Err(e) => return Err(format!("importing account {name:?}: {e}")),
        }
    }

    let imported_path = data_dir.join("accounts.json.imported");
    // `rename` refuses an existing destination on Windows; a stale marker
    // from an interrupted earlier import may be replaced.
    let _ = std::fs::remove_file(&imported_path);
    std::fs::rename(&path, &imported_path)
        .map_err(|e| format!("renaming {} after import: {e}", path.display()))?;
    tracing::info!(
        imported,
        from = %path.display(),
        to = %imported_path.display(),
        "legacy accounts.json imported into the store"
    );
    Ok(imported)
}

// ---------------------------------------------------------------------------
// Validation + hashing
// ---------------------------------------------------------------------------

/// `spawn_blocking` with the join failure folded into [`CreateErr`].
///
/// # Errors
///
/// [`CreateErr::Io`] if the blocking task panics or is cancelled.
pub async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CreateErr> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CreateErr::Io(format!("blocking task failed: {e}")))
}

/// Account name rule: 3–20 chars of `[A-Za-z0-9_-]`, not reserved.
///
/// # Errors
///
/// [`CreateErr::BadAccountName`] / [`CreateErr::ReservedName`].
pub fn validate_account_name(name: &str) -> Result<(), CreateErr> {
    if !(3..=20).contains(&name.len())
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(CreateErr::BadAccountName);
    }
    if RESERVED_NAMES.contains(&name.to_lowercase().as_str()) {
        return Err(CreateErr::ReservedName);
    }
    Ok(())
}

/// Character name rule: starts with a letter, 2–20 chars of
/// `[A-Za-z0-9_-]`, not reserved. (The world additionally rejects names
/// already held by any actor.)
///
/// # Errors
///
/// [`CreateErr::BadCharacterName`] / [`CreateErr::ReservedName`].
pub fn validate_character_name(name: &str) -> Result<(), CreateErr> {
    if !(2..=20).contains(&name.len())
        || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(CreateErr::BadCharacterName);
    }
    if RESERVED_NAMES.contains(&name.to_lowercase().as_str()) {
        return Err(CreateErr::ReservedName);
    }
    Ok(())
}

/// Password rule: 4–128 characters. Shared by account creation and
/// `@newpassword`.
///
/// # Errors
///
/// [`CreateErr::BadPassword`].
pub fn validate_password(password: &str) -> Result<(), CreateErr> {
    if (4..=128).contains(&password.len()) {
        Ok(())
    } else {
        Err(CreateErr::BadPassword)
    }
}

/// argon2id-hash a password into a PHC string (default params: Argon2id
/// v19, m=19456 KiB, t=2, p=1 — the crate's OWASP-aligned defaults).
/// Public so tests and seeding tools can mint valid account records.
///
/// Salt: 16 bytes from `rand`'s thread-local CSPRNG (the `argon2` crate's
/// own `OsRng` path is feature-gated off in this workspace's dep graph).
///
/// # Errors
///
/// [`CreateErr::Io`] on a hashing/encoding failure.
pub fn hash_password(password: &str) -> Result<String, CreateErr> {
    let salt_bytes: [u8; 16] = rand::random();
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| CreateErr::Io(e.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| CreateErr::Io(e.to_string()))
}

/// Verify `password` against a stored PHC string.
#[must_use]
pub fn verify_hash(phc: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        tracing::error!("stored password hash is not a valid PHC string");
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Burn one verify against a fixed hash so "no such account" costs the same
/// as "wrong password".
pub fn burn_dummy_verify(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let dummy = DUMMY.get_or_init(|| {
        hash_password("timing-pad-not-a-real-password").expect("dummy hash must succeed")
    });
    let _ = verify_hash(dummy, password);
}
#[cfg(test)]
mod tests {
    use super::*;
    use mush_store::InMemoryStore;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch_dir(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mushd-accounts-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn account_name_rules() {
        for good in ["nick", "Nick_York", "a-b-3", "abc"] {
            assert_eq!(validate_account_name(good), Ok(()), "{good:?}");
        }
        for bad in [
            "ab",
            "",
            "有名",
            "nick york",
            "a".repeat(21).as_str(),
            "n!ck",
        ] {
            assert_eq!(
                validate_account_name(bad),
                Err(CreateErr::BadAccountName),
                "{bad:?}"
            );
        }
        for reserved in ["God", "PLAY", "guest", "new", "root", "Admin"] {
            assert_eq!(
                validate_account_name(reserved),
                Err(CreateErr::ReservedName),
                "{reserved:?}"
            );
        }
    }

    #[test]
    fn character_name_rules() {
        for good in ["Ni", "Nick", "X-9", "Zo_e"] {
            assert_eq!(validate_character_name(good), Ok(()), "{good:?}");
        }
        for bad in ["N", "9lives", "_x", "Nick Y", ""] {
            assert_eq!(
                validate_character_name(bad),
                Err(CreateErr::BadCharacterName),
                "{bad:?}"
            );
        }
        assert_eq!(
            validate_character_name("Guest"),
            Err(CreateErr::ReservedName)
        );
    }

    #[tokio::test]
    async fn import_moves_legacy_accounts_into_the_store() {
        let dir = scratch_dir("import");
        let hash = hash_password("hunter42").unwrap();
        let json = serde_json::json!({
            "version": 1,
            "accounts": [
                { "name": "nick", "hash": hash,
                  "characters": [ { "id": 7, "name": "Nick" } ] },
                { "name": "dupe", "hash": hash, "characters": [] },
            ]
        });
        std::fs::write(dir.join("accounts.json"), json.to_string()).unwrap();

        let store = InMemoryStore::new();
        // "dupe" already lives in the store: import must keep the store copy.
        store
            .create_account(AccountRec {
                name: "Dupe".into(),
                argon2_hash: "store-copy".into(),
                created: 1,
                characters: vec![],
                ssh_keys: vec![],
            })
            .await
            .unwrap();

        let imported = import_accounts_json(&store, &dir).await.unwrap();
        assert_eq!(imported, 1);
        assert!(!dir.join("accounts.json").exists());
        assert!(dir.join("accounts.json.imported").exists());

        let nick = store.get_account("NICK").await.unwrap().unwrap();
        assert_eq!(nick.name, "nick");
        assert_eq!(nick.characters.len(), 1);
        assert_eq!(nick.characters[0].name, "Nick");
        assert!(verify_hash(&nick.argon2_hash, "hunter42"));
        let dupe = store.get_account("dupe").await.unwrap().unwrap();
        assert_eq!(dupe.argon2_hash, "store-copy", "store copy must win");

        // Second boot: nothing to import.
        assert_eq!(import_accounts_json(&store, &dir).await.unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn import_corrupt_file_is_a_boot_error() {
        let dir = scratch_dir("import-corrupt");
        std::fs::write(dir.join("accounts.json"), "{not json").unwrap();
        let store = InMemoryStore::new();
        assert!(import_accounts_json(&store, &dir).await.is_err());
        // The corrupt file is left in place for the operator.
        assert!(dir.join("accounts.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
