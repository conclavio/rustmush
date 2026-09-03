//! Store-backed accounts (M2): argon2id verification against
//! `account:{name}` docs in the [`ObjectStore`], character creation through
//! the world task, and the one-time import of the M1 `accounts.json`.
//!
//! The [`AuthBackend`] trait is the seam the SSH bridge talks through;
//! [`Accounts`] is the production implementation over `Arc<dyn
//! ObjectStore>` (Redis in production, [`mush_store::InMemoryStore`] in
//! tests). Passwords are argon2id PHC strings (design 02 §8.1); hashing and
//! verification stay caller-side — the store only ever holds the opaque
//! hash. Verification is CPU-heavy by design (~tens of milliseconds), so it
//! runs on the blocking pool internally; callers just `.await`.
//!
//! Creating an account **never touches `World`**: the flow is
//!
//! 1. validate names + password, pre-check the character name against the
//!    world's player index ([`WorldHandle::name_free`]);
//! 2. reserve the account record — [`ObjectStore::create_account`] is
//!    server-side atomic, so concurrent creates race safely;
//! 3. ask the world task to create the Player
//!    ([`WorldHandle::create_player`]), which **waits for the store to
//!    acknowledge the creation batch** (durable write-through);
//! 4. link the character ([`ObjectStore::link_character`], atomic append).
//!
//! Only after all of that does the caller emit its confirmation output —
//! the ack gate PLAN's write-through design requires. The store has no
//! account deletion yet (a later milestone), so a world-side name conflict
//! that slips past the pre-check (step 1 is advisory, not atomic) leaves a
//! character-less account behind; it is logged loudly and the pre-check
//! makes it a genuine race, not a normal path.
//!
//! ## `accounts.json` import (one-time)
//!
//! If `data_dir/accounts.json` (the M1 interim store) exists at boot,
//! [`import_accounts_json`] copies every record into the store (existing
//! store accounts win) and renames the file to `accounts.json.imported` so
//! the import never runs twice.

use std::sync::Arc;

use mush_config::accounts::{
    burn_dummy_verify, hash_password, run_blocking, validate_account_name, validate_character_name,
    validate_password, verify_hash,
};
use mush_config::unix_now_s;
use mush_ipc::{CreatePlayerError, GatewayLink, ServiceReq, ServiceResp};
use mush_store::{AccountRec, ObjectStore, StoreError};

// The transport-agnostic account vocabulary lives in `mush-config`; re-export it
// here so existing `…accounts::{CharSummary, CreateErr, …}` imports keep working.
pub use mush_config::accounts::{CharSummary, CreateErr, RESERVED_NAMES};

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

/// The pluggable auth seam the SSH bridge is generic over.
///
/// Methods are declared as `-> impl Future + Send` (not `async fn`) so
/// generic callers can box them into `Send` futures — the SSH bridge
/// implements the dyn-compatible `mush_net_ssh::SshHooks` on top of a
/// generic `A: AuthBackend`. Implementors still just write `async` blocks.
pub trait AuthBackend: Send + Sync {
    /// Check credentials. Internally CPU-heavy (argon2id, run on the
    /// blocking pool); callers just await.
    fn verify(&self, account: &str, password: &str) -> impl Future<Output = AuthOutcome> + Send;
    /// Authenticate an account by an offered SSH public key (M19). The
    /// offered key arrives as an OpenSSH `"type base64 comment"` line;
    /// implementors parse it and each of the account's registered keys and
    /// compare by key material (never raw string), returning
    /// [`AuthOutcome::Ok`] on a match and [`AuthOutcome::BadCredentials`] on
    /// an unknown account *or* no match (uniform — no existence oracle). A
    /// public key holds no secret, so there is no argon2/dummy-burn timing
    /// concern here. The default rejects, so a backend that has not wired
    /// key auth safely falls back to password.
    fn verify_pubkey(
        &self,
        _account: &str,
        _offered_openssh: &str,
    ) -> impl Future<Output = AuthOutcome> + Send {
        async { AuthOutcome::BadCredentials }
    }
    /// The characters on an account (empty for unknown accounts).
    fn characters(&self, account: &str) -> impl Future<Output = Vec<CharSummary>> + Send;
    /// Create an account with its first character. The Player object is
    /// created by the world task and the confirmation is gated on the
    /// store's ack.
    fn create(
        &self,
        account: &str,
        password: &str,
        char_name: &str,
    ) -> impl Future<Output = Result<CharSummary, CreateErr>> + Send;
    /// Create an additional character on an existing account (`newchar` on
    /// the character-select screen), honoring the per-account cap. Same ack
    /// gating as [`AuthBackend::create`].
    fn create_character(
        &self,
        account: &str,
        char_name: &str,
    ) -> impl Future<Output = Result<CharSummary, CreateErr>> + Send;
}

/// The production [`AuthBackend`]: the object store plus the world-task
/// handle for Player creation.
pub struct Accounts {
    store: Arc<dyn ObjectStore>,
    /// The protocol-typed handle to the server half (M16a): character
    /// creation and name pre-checks cross the boundary as `mush-ipc`
    /// [`ServiceReq`]s through this trait object, never by touching `World`.
    link: Arc<dyn GatewayLink>,
    max_characters: usize,
}

impl Accounts {
    /// Wire the store to the world (any [`GatewayLink`], e.g. the in-process
    /// `mush_server::world_task::WorldHandle`). `max_characters` is the
    /// `[game].max_characters` cap (per account).
    #[must_use]
    pub fn new(
        store: Arc<dyn ObjectStore>,
        world: impl GatewayLink + 'static,
        max_characters: usize,
    ) -> Accounts {
        Accounts::with_link(store, Arc::new(world), max_characters)
    }

    /// Wire the store to an explicit [`GatewayLink`] (M16b): the forked
    /// `mush-gateway` passes a socket-client link so Player creation crosses to
    /// the server process as a `mush-ipc` [`ServiceReq`], never touching a local
    /// `World`.
    #[must_use]
    pub fn with_link(
        store: Arc<dyn ObjectStore>,
        link: Arc<dyn GatewayLink>,
        max_characters: usize,
    ) -> Accounts {
        Accounts {
            store,
            link,
            max_characters,
        }
    }

    /// Advisory name check with transport failures kept distinct from the
    /// authoritative "taken" answer. In particular, bounded IPC overload must
    /// not masquerade as `NameFree(false)` and leak into player-facing state as
    /// a false collision.
    async fn require_name_free(&self, char_name: &str) -> Result<(), CreateErr> {
        match self
            .link
            .request(ServiceReq::NameFree {
                name: char_name.to_string(),
            })
            .await
        {
            Some(ServiceResp::NameFree(true)) => Ok(()),
            Some(ServiceResp::NameFree(false)) => Err(CreateErr::CharacterNameTaken),
            Some(ServiceResp::Busy) => Err(CreateErr::World(
                "the game server is busy; please try again shortly".to_string(),
            )),
            Some(ServiceResp::Unavailable) | None => {
                Err(CreateErr::World(CreatePlayerError::WorldGone.to_string()))
            }
            Some(other) => Err(CreateErr::World(format!(
                "unexpected name-check response: {other:?}"
            ))),
        }
    }

    /// Fetch an account doc, mapping store failures to a logged `None`
    /// (auth paths fail closed and uniform).
    async fn fetch(&self, account: &str) -> Option<AccountRec> {
        match self.store.get_account(account).await {
            Ok(rec) => rec,
            Err(e) => {
                tracing::error!(account, error = %e, "account fetch failed");
                None
            }
        }
    }

    /// The shared tail of both creation flows: Player via the world task
    /// (ack-gated), then the atomic link.
    async fn create_and_link(
        &self,
        account: &str,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        let id = match self
            .link
            .request(ServiceReq::CreatePlayer {
                name: char_name.to_string(),
            })
            .await
        {
            Some(ServiceResp::CreatePlayer(Ok(id))) => id,
            Some(ServiceResp::CreatePlayer(Err(CreatePlayerError::NameTaken))) => {
                // The pre-check raced another create. The account (possibly
                // freshly reserved, possibly character-less) stays — the
                // store cannot delete accounts yet. Loud log, clear error.
                tracing::error!(
                    account,
                    char_name,
                    "character name taken after pre-check; account left without this character"
                );
                return Err(CreateErr::CharacterNameTaken);
            }
            Some(ServiceResp::CreatePlayer(Err(e))) => return Err(CreateErr::World(e.to_string())),
            Some(ServiceResp::Busy) => {
                return Err(CreateErr::World(
                    "the game server is busy; please try again shortly".to_string(),
                ));
            }
            Some(ServiceResp::Unavailable) => {
                return Err(CreateErr::World(CreatePlayerError::WorldGone.to_string()));
            }
            // The world is gone or answered unexpectedly: same failure the
            // direct `WorldGone` path reported.
            _ => return Err(CreateErr::World(CreatePlayerError::WorldGone.to_string())),
        };
        match self.store.link_character(account, id, char_name).await {
            Ok(()) => Ok(CharSummary {
                id,
                name: char_name.to_string(),
            }),
            Err(e) => {
                // The Player exists but the account does not list it —
                // recoverable by staff, never silent.
                tracing::error!(account, %id, char_name, error = %e, "linking character failed");
                Err(CreateErr::Io(e.to_string()))
            }
        }
    }
}

impl AuthBackend for Accounts {
    async fn verify(&self, account: &str, password: &str) -> AuthOutcome {
        let Some(rec) = self.fetch(account).await else {
            // A miss still burns one argon2 verify so timing does not
            // reveal account existence.
            let password = password.to_string();
            let _ = run_blocking(move || burn_dummy_verify(&password)).await;
            return AuthOutcome::BadCredentials;
        };
        let ok = {
            let (hash, password) = (rec.argon2_hash.clone(), password.to_string());
            run_blocking(move || verify_hash(&hash, &password))
                .await
                .unwrap_or(false)
        };
        if ok {
            AuthOutcome::Ok {
                account: rec.name,
                characters: rec.characters.into_iter().map(Into::into).collect(),
            }
        } else {
            AuthOutcome::BadCredentials
        }
    }

    async fn verify_pubkey(&self, account: &str, offered_openssh: &str) -> AuthOutcome {
        use russh::keys::ssh_key::PublicKey;
        // Read the account FRESH per attempt (no cache), so an in-game
        // `@sshkey/add` authenticates on the very next `ssh -i` (invariant
        // S4). Unknown account and no-match both return BadCredentials with
        // no observable early branch (invariant S1) — a public key holds no
        // secret, so there is no argon2/dummy-burn timing to preserve.
        let Some(rec) = self.fetch(account).await else {
            return AuthOutcome::BadCredentials;
        };
        let Ok(offered) = PublicKey::from_openssh(offered_openssh) else {
            return AuthOutcome::BadCredentials;
        };
        // Compare by key material (comment-independent), never by raw string.
        let matched = rec.ssh_keys.iter().any(|stored| {
            PublicKey::from_openssh(&stored.openssh)
                .is_ok_and(|pk| pk.key_data() == offered.key_data())
        });
        if matched {
            AuthOutcome::Ok {
                account: rec.name,
                characters: rec.characters.into_iter().map(Into::into).collect(),
            }
        } else {
            AuthOutcome::BadCredentials
        }
    }

    async fn characters(&self, account: &str) -> Vec<CharSummary> {
        self.fetch(account)
            .await
            .map(|r| r.characters.into_iter().map(Into::into).collect())
            .unwrap_or_default()
    }

    async fn create(
        &self,
        account: &str,
        password: &str,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        validate_account_name(account)?;
        validate_password(password)?;
        validate_character_name(char_name)?;
        // Advisory pre-check: catch a taken character name BEFORE the
        // account record exists (the world create re-validates atomically).
        self.require_name_free(char_name).await?;
        let hash = {
            let password = password.to_string();
            run_blocking(move || hash_password(&password)).await??
        };
        let rec = AccountRec {
            name: account.to_string(),
            argon2_hash: hash,
            created: unix_now_s(),
            characters: Vec::new(),
            ssh_keys: Vec::new(),
        };
        match self.store.create_account(rec).await {
            Ok(()) => {}
            Err(StoreError::AccountExists) => return Err(CreateErr::AccountExists),
            Err(e) => return Err(CreateErr::Io(e.to_string())),
        }
        self.create_and_link(account, char_name).await
    }

    async fn create_character(
        &self,
        account: &str,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        validate_character_name(char_name)?;
        let rec = self.fetch(account).await.ok_or(CreateErr::NoSuchAccount)?;
        // Known TOCTOU: two concurrent `newchar`s on the SAME account at
        // cap-1 can both pass this check (link_character appends
        // unconditionally), exceeding the cap by one. Closing it needs a
        // server-side conditional append (WATCH/MULTI or Lua); accepted for
        // M2 — it takes the account holder racing themselves, and the only
        // damage is one extra character.
        if rec.characters.len() >= self.max_characters {
            return Err(CreateErr::CharacterLimit);
        }
        self.require_name_free(char_name).await?;
        self.create_and_link(account, char_name).await
    }
}

#[cfg(test)]
mod pubkey_tests {
    use super::*;
    use futures::future::BoxFuture;
    use mush_ipc::ToServer;
    use mush_store::{InMemoryStore, SshKeyRec};

    // A fixed ed25519 public line (its private half drives the mush-net-ssh
    // loopback e2e). Comparison is by key material, so a copy under a
    // different comment must still match.
    const PUB_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICKZsuYnz5vDjbi8gtQSL5djhsj1/8olA+yHkPMLp/Ki loopback-test@rustmush";
    const PUB_A_RENAMED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICKZsuYnz5vDjbi8gtQSL5djhsj1/8olA+yHkPMLp/Ki someone@elsewhere";
    // A different key entirely.
    const PUB_B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMiSXEiwVn9lp2CB6rnYzCoM79r94VldH/pvhWAc9wRH wrong@rustmush";

    /// verify_pubkey never touches the world link; a no-op stub suffices.
    struct NoLink;
    impl GatewayLink for NoLink {
        fn to_server(&self, _msg: ToServer) -> BoxFuture<'_, bool> {
            Box::pin(async { false })
        }
        fn request(&self, _req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
            Box::pin(async { None })
        }
    }

    fn fp(line: &str) -> String {
        russh::keys::ssh_key::PublicKey::from_openssh(line)
            .unwrap()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string()
    }

    #[tokio::test]
    async fn verify_pubkey_matches_by_key_material_uniform_on_miss() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        store
            .create_account(AccountRec {
                name: "Nick".into(),
                argon2_hash: "x".into(),
                created: unix_now_s(),
                characters: Vec::new(),
                ssh_keys: Vec::new(),
            })
            .await
            .unwrap();
        let accounts = Accounts::with_link(Arc::clone(&store), Arc::new(NoLink), 3);

        // No keys registered yet: even the eventual "right" key is refused.
        assert_eq!(
            accounts.verify_pubkey("nick", PUB_A).await,
            AuthOutcome::BadCredentials
        );

        // Register PUB_A; the next verify reads the account FRESH (S4).
        let rec = SshKeyRec {
            openssh: PUB_A.into(),
            fingerprint: fp(PUB_A),
            added: unix_now_s(),
        };
        assert!(
            store.set_account_ssh_keys("nick", &[rec]).await.unwrap(),
            "the account exists, so the write reports true"
        );

        // Exact line matches; the SAME key under a different comment matches
        // too — comment-independent key_data compare (S1). The account name
        // comes back in its display case.
        assert!(matches!(
            accounts.verify_pubkey("nick", PUB_A).await,
            AuthOutcome::Ok { ref account, .. } if account == "Nick"
        ));
        assert!(matches!(
            accounts.verify_pubkey("nick", PUB_A_RENAMED).await,
            AuthOutcome::Ok { .. }
        ));

        // A different key does not match; an unknown account and a malformed
        // offered line are the SAME BadCredentials (no existence oracle, no
        // panic).
        assert_eq!(
            accounts.verify_pubkey("nick", PUB_B).await,
            AuthOutcome::BadCredentials
        );
        assert_eq!(
            accounts.verify_pubkey("ghost", PUB_A).await,
            AuthOutcome::BadCredentials
        );
        assert_eq!(
            accounts.verify_pubkey("nick", "not-a-key").await,
            AuthOutcome::BadCredentials
        );
    }
}
