//! Minimal account storage for M1 — **interim by design**.
//!
//! Accounts live in a [`DashMap`] mirrored to `data_dir/accounts.json`
//! (atomic temp-file+rename writes), loaded once at boot. **M2 replaces this
//! wholesale with the Redis store** (`account:{name}` JSON docs per
//! `docs/PLAN.md`); the [`AuthBackend`] trait is the seam the swap happens
//! behind, which is why nothing outside this module names [`AccountStore`]
//! directly.
//!
//! Passwords are argon2id PHC strings (design 02 §8.1). Verification is
//! CPU-heavy by design (~tens of milliseconds): callers on async threads
//! wrap [`AuthBackend::verify`] in `spawn_blocking` (the SSH bridge does).
//!
//! Creating an account **never touches `World`**: the [`Accounts`] service
//! reserves the account record, asks the world task to create the Player via
//! [`WorldHandle::create_player`] (a typed message with a oneshot reply),
//! then links the resulting id — rolling the reservation back if the world
//! says no.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use dashmap::DashMap;
use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::world_task::{CreatePlayerErr, WorldHandle};

/// Account names no one may register (routing usernames, staff identities).
pub const RESERVED_NAMES: &[&str] = &["god", "play", "guest", "new", "root", "admin"];

/// One character on an account. The name is denormalized from the world at
/// link time (M1-interim; Redis owns this in M2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharSummary {
    /// The character's object id.
    pub id: ObjId,
    /// The character's name when it was linked.
    pub name: String,
}

/// Result of a password check. Unknown account and wrong password are the
/// same variant on purpose — the classic uniform login error (design 02
/// §5.3) leaks nothing about which accounts exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthOutcome {
    /// Credentials good; here are the account's characters.
    Ok {
        /// Account name in its registered display case.
        account: String,
        /// The account's characters.
        characters: Vec<CharSummary>,
    },
    /// Unknown account *or* wrong password.
    BadCredentials,
}

/// Account-creation failure (player-facing messages).
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
    /// The account file could not be written.
    #[error("account storage failed: {0}")]
    Io(String),
}

/// The pluggable auth seam: M1 implements it with [`Accounts`]
/// (JSON-file-backed); M2 swaps in the Redis-backed store.
///
/// `create` is declared as `-> impl Future + Send` (not `async fn`) so
/// generic callers can box it into `Send` futures — the SSH bridge
/// implements the dyn-compatible `mush_net_ssh::SshHooks` on top of a
/// generic `A: AuthBackend`. Implementors still just write `async fn`.
pub trait AuthBackend: Send + Sync {
    /// Check credentials. CPU-heavy (argon2id) — call via `spawn_blocking`
    /// from async contexts.
    fn verify(&self, account: &str, password: &str) -> AuthOutcome;
    /// The characters on an account (empty for unknown accounts).
    fn characters(&self, account: &str) -> Vec<CharSummary>;
    /// Create an account with its first character. The Player object is
    /// created by the world task; on failure nothing is left behind.
    fn create(
        &self,
        account: &str,
        password: &str,
        char_name: &str,
    ) -> impl Future<Output = Result<CharSummary, CreateErr>> + Send;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccountRecord {
    /// Display-case name (the map key is the lowercase form).
    name: String,
    /// argon2id PHC string.
    hash: String,
    /// Linked characters.
    characters: Vec<CharSummary>,
}

#[derive(Serialize, Deserialize)]
struct AccountsFile {
    version: u32,
    accounts: Vec<AccountRecord>,
}

/// The M1 record store: in-memory map + JSON file. See the module docs.
pub struct AccountStore {
    path: PathBuf,
    map: DashMap<String, AccountRecord>,
    /// Serializes file writes (the map itself is shard-locked).
    save: Mutex<()>,
}

impl AccountStore {
    /// Open the store in `data_dir`, loading `accounts.json` if present
    /// (creating the directory if needed).
    pub fn open(data_dir: &Path) -> std::io::Result<AccountStore> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("accounts.json");
        let map = DashMap::new();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file: AccountsFile = serde_json::from_str(&text).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{} is corrupt: {e}", path.display()),
                    )
                })?;
                for rec in file.accounts {
                    map.insert(rec.name.to_lowercase(), rec);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        tracing::info!(path = %path.display(), accounts = map.len(), "account store open");
        Ok(AccountStore {
            path,
            map,
            save: Mutex::new(()),
        })
    }

    /// Number of accounts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the store has no accounts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Check `password` against `account`. Uniform failure; a miss still
    /// burns one argon2 verify so timing does not reveal account existence.
    #[must_use]
    pub fn verify(&self, account: &str, password: &str) -> AuthOutcome {
        let key = account.to_lowercase();
        let Some(rec) = self.map.get(&key).map(|r| r.clone()) else {
            burn_dummy_verify(password);
            return AuthOutcome::BadCredentials;
        };
        if verify_hash(&rec.hash, password) {
            AuthOutcome::Ok {
                account: rec.name,
                characters: rec.characters,
            }
        } else {
            AuthOutcome::BadCredentials
        }
    }

    /// The characters on an account (empty when unknown).
    #[must_use]
    pub fn characters(&self, account: &str) -> Vec<CharSummary> {
        self.map
            .get(&account.to_lowercase())
            .map(|r| r.characters.clone())
            .unwrap_or_default()
    }

    /// Validate names and password, hash, and insert an account with no
    /// characters yet. Persists. The caller links the first character next
    /// (or [`AccountStore::unreserve`]s on failure).
    pub fn reserve(&self, account: &str, password: &str) -> Result<(), CreateErr> {
        validate_account_name(account)?;
        validate_password(password)?;
        let hash = hash_password(password)?;
        let key = account.to_lowercase();
        // Entry-based insert so two concurrent creates cannot both win.
        match self.map.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(_) => return Err(CreateErr::AccountExists),
            dashmap::mapref::entry::Entry::Vacant(v) => {
                v.insert(AccountRecord {
                    name: account.to_string(),
                    hash,
                    characters: Vec::new(),
                });
            }
        }
        if let Err(e) = self.persist() {
            self.map.remove(&account.to_lowercase());
            return Err(CreateErr::Io(e.to_string()));
        }
        Ok(())
    }

    /// Link a character to an account. Persist failures are logged, not
    /// fatal — the in-memory link is authoritative for this process
    /// (M1-interim tradeoff).
    pub fn link_character(&self, account: &str, id: ObjId, name: &str) {
        if let Some(mut rec) = self.map.get_mut(&account.to_lowercase()) {
            rec.characters.push(CharSummary {
                id,
                name: name.to_string(),
            });
        } else {
            tracing::error!(account, %id, "link_character: no such account");
            return;
        }
        if let Err(e) = self.persist() {
            tracing::error!(account, error = %e, "persisting character link failed");
        }
    }

    /// Roll back a reservation (world-side creation failed). Persists.
    pub fn unreserve(&self, account: &str) {
        if self.map.remove(&account.to_lowercase()).is_some()
            && let Err(e) = self.persist()
        {
            tracing::error!(account, error = %e, "persisting unreserve failed");
        }
    }

    /// Write the whole store to `accounts.json` atomically: serialize to
    /// `accounts.json.tmp`, then rename over the real file (`rename`
    /// replaces on both Windows and POSIX).
    fn persist(&self) -> std::io::Result<()> {
        let _guard = self.save.lock().expect("account save lock poisoned");
        let mut accounts: Vec<AccountRecord> = self.map.iter().map(|r| r.clone()).collect();
        accounts.sort_by_key(|a| a.name.to_lowercase());
        let file = AccountsFile {
            version: 1,
            accounts,
        };
        let json = serde_json::to_string_pretty(&file).expect("account records serialize");
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)
    }
}

/// The M1 [`AuthBackend`]: [`AccountStore`] + the world-task handle for
/// Player creation. The store rides an `Arc` so the create path can hop to
/// the blocking pool (argon2id hash + whole-file persist are too heavy for
/// tokio workers).
pub struct Accounts {
    store: Arc<AccountStore>,
    world: WorldHandle,
}

impl Accounts {
    /// Wire the store to the world task.
    #[must_use]
    pub fn new(store: AccountStore, world: WorldHandle) -> Accounts {
        Accounts {
            store: Arc::new(store),
            world,
        }
    }
}

impl AuthBackend for Accounts {
    fn verify(&self, account: &str, password: &str) -> AuthOutcome {
        self.store.verify(account, password)
    }

    fn characters(&self, account: &str) -> Vec<CharSummary> {
        self.store.characters(account)
    }

    async fn create(
        &self,
        account: &str,
        password: &str,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        validate_character_name(char_name)?;
        // 1. Reserve the account (validates account name + password, argon2
        //    hash, file persist) — CPU + disk heavy, so blocking pool.
        {
            let store = Arc::clone(&self.store);
            let (account, password) = (account.to_string(), password.to_string());
            run_blocking(move || store.reserve(&account, &password)).await??;
        }
        // 2. World task creates the Player (typed request, oneshot reply).
        match self.world.create_player(char_name).await {
            Ok(id) => {
                // 3. Link (persists; failure inside is logged, not fatal).
                let store = Arc::clone(&self.store);
                let (account, name) = (account.to_string(), char_name.to_string());
                run_blocking(move || store.link_character(&account, id, &name)).await?;
                Ok(CharSummary {
                    id,
                    name: char_name.to_string(),
                })
            }
            Err(e) => {
                let store = Arc::clone(&self.store);
                let account = account.to_string();
                run_blocking(move || store.unreserve(&account)).await?;
                Err(match e {
                    CreatePlayerErr::NameTaken => CreateErr::CharacterNameTaken,
                    other => CreateErr::World(other.to_string()),
                })
            }
        }
    }
}

/// `spawn_blocking` with the join failure folded into [`CreateErr`].
async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CreateErr> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CreateErr::Io(format!("blocking task failed: {e}")))
}

/// Account name rule: 3–20 chars of `[A-Za-z0-9_-]`, not reserved.
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

fn validate_password(password: &str) -> Result<(), CreateErr> {
    if (4..=128).contains(&password.len()) {
        Ok(())
    } else {
        Err(CreateErr::BadPassword)
    }
}

/// argon2id-hash a password into a PHC string (default params: Argon2id
/// v19, m=19456 KiB, t=2, p=1 — the crate's OWASP-aligned defaults).
///
/// Salt: 16 bytes from `rand`'s thread-local CSPRNG (the `argon2` crate's
/// own `OsRng` path is feature-gated off in this workspace's dep graph).
fn hash_password(password: &str) -> Result<String, CreateErr> {
    let salt_bytes: [u8; 16] = rand::random();
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| CreateErr::Io(e.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| CreateErr::Io(e.to_string()))
}

fn verify_hash(phc: &str, password: &str) -> bool {
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
fn burn_dummy_verify(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let dummy = DUMMY.get_or_init(|| {
        hash_password("timing-pad-not-a-real-password").expect("dummy hash must succeed")
    });
    let _ = verify_hash(dummy, password);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch_dir(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mushd-accounts-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn reserve_verify_and_uniform_failure() {
        let dir = scratch_dir("verify");
        let store = AccountStore::open(&dir).unwrap();
        store.reserve("nick", "hunter42").unwrap();

        match store.verify("Nick", "hunter42") {
            AuthOutcome::Ok {
                account,
                characters,
            } => {
                assert_eq!(account, "nick");
                assert!(characters.is_empty());
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        // Wrong password and unknown account are indistinguishable.
        assert_eq!(store.verify("nick", "wrong"), AuthOutcome::BadCredentials);
        assert_eq!(
            store.verify("nobody", "hunter42"),
            AuthOutcome::BadCredentials
        );
        // Duplicate reservation (any case) refused; bad password refused.
        assert_eq!(
            store.reserve("NICK", "otherpw"),
            Err(CreateErr::AccountExists)
        );
        assert_eq!(store.reserve("newbie", "abc"), Err(CreateErr::BadPassword));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persistence_round_trip_is_atomic() {
        let dir = scratch_dir("persist");
        {
            let store = AccountStore::open(&dir).unwrap();
            store.reserve("nick", "hunter42").unwrap();
            store.link_character("nick", ObjId(7), "Nick");
            store.reserve("gone", "hunter42").unwrap();
            store.unreserve("gone");
        }
        // No temp file left behind; the real file exists.
        assert!(dir.join("accounts.json").exists());
        assert!(!dir.join("accounts.json.tmp").exists());

        // A fresh store sees exactly what was persisted.
        let store = AccountStore::open(&dir).unwrap();
        assert_eq!(store.len(), 1);
        match store.verify("nick", "hunter42") {
            AuthOutcome::Ok { characters, .. } => {
                assert_eq!(
                    characters,
                    vec![CharSummary {
                        id: ObjId(7),
                        name: "Nick".to_string()
                    }]
                );
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        assert_eq!(store.characters("NICK")[0].name, "Nick");
        assert!(store.characters("gone").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_a_boot_error() {
        let dir = scratch_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("accounts.json"), "{not json").unwrap();
        assert!(AccountStore::open(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
