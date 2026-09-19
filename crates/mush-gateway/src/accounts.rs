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
//! 4. link the character ([`ObjectStore::link_character`], atomic append), and
//!    on failure take the mapping step 3 put in the server's live account
//!    oracle back out again ([`ServiceReq::UnlinkPlayer`], 16b cut F.4), so the
//!    orphan Player is an orphan on both sides.
//!
//! Steps 1 and 3 are the two requests that cross the boundary, and both wait out
//! the world's one transient refusal rather than reporting it (16b cut F.4 follow-up):
//! see [`COMMIT_RETRY_ATTEMPTS`].
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
use std::time::Duration;

use tokio::time::Instant;

use mush_config::accounts::{
    burn_dummy_verify, hash_password, run_blocking, validate_account_name, validate_character_name,
    validate_password, verify_hash,
};
use mush_config::unix_now_s;
use mush_core::ObjId;
use mush_ipc::{CreatePlayerError, GatewayLink, ServiceReq, ServiceResp};
use mush_store::{AccountRec, ObjectStore, StoreError};

// The transport-agnostic account vocabulary lives in `mush-config`; re-export it
// here so existing `…accounts::{CharSummary, CreateErr, …}` imports keep working.
pub use mush_config::accounts::{CharSummary, CreateErr, RESERVED_NAMES};

/// **Waiting out a commit** (16b cut F.4 follow-up).
///
/// The world refuses reads and new authoritative work with [`ServiceResp::Busy`]
/// for as long as one commit is awaiting its exact durable receipt — the world
/// loop's `RefusalReason::CommitPending` fan-out, `WorldMode::CommitPending`. It
/// is a transient that says nothing at all about the request, and **boot is
/// exactly when a registration meets it**: the world is committing the layers it
/// has just built while the first player is typing `create` at the welcome
/// screen. Reporting it there spends a typed password on a refusal the player
/// can do nothing about but type the whole line again.
///
/// So the two requests the account flow sends are re-sent while that answer
/// stands, bounded the way `bridge_common::delivery_barrier` bounds the same
/// answer: a fixed number of re-sends inside one total deadline, after which the
/// Busy is reported exactly as it was before. On an idle boundary the count binds
/// first (forty re-sends, so about four seconds of waiting); the deadline is the
/// backstop for a boundary whose own requests are slow, because a wedged world
/// must never hold a welcome screen open.
const COMMIT_RETRY_ATTEMPTS: usize = 40;
/// How long the flow waits between re-sends.
const COMMIT_RETRY_DELAY: Duration = Duration::from_millis(100);
/// The whole budget, counted on tokio's clock so a paused-time test of it costs
/// no wall clock.
const COMMIT_RETRY_TIMEOUT: Duration = Duration::from_secs(5);

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

    /// One request to the server half, re-sent while the world answers the
    /// transient commit refusal (16b cut F.4 follow-up; see [`COMMIT_RETRY_ATTEMPTS`]).
    ///
    /// Only the two requests of the creation flow take this door, and both are
    /// safe to re-send: [`ServiceReq::NameFree`] is an advisory read that touches
    /// nothing, and a Busy answer to [`ServiceReq::CreatePlayer`] is documented
    /// as refused **before touching world state**
    /// (`mush_server::world_task::CreatePlayerErr::Busy`), so a re-send cannot
    /// make two Players. Every other answer is handed straight back on the first
    /// try — and so is a Busy that outlives the budget, so each caller's own
    /// mapping of the answers is unchanged.
    async fn request_past_commit(&self, req: ServiceReq) -> Option<ServiceResp> {
        let deadline = Instant::now() + COMMIT_RETRY_TIMEOUT;
        let mut answer = self.link.request(req.clone()).await;
        let mut attempts = 1;
        while matches!(answer, Some(ServiceResp::Busy)) && attempts < COMMIT_RETRY_ATTEMPTS {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(COMMIT_RETRY_DELAY.min(remaining)).await;
            answer = self.link.request(req.clone()).await;
            attempts += 1;
        }
        if matches!(answer, Some(ServiceResp::Busy)) {
            tracing::warn!(
                request = ?req,
                attempts,
                "the world stayed mid-commit across the whole retry budget; the request is \
                 refused as busy"
            );
        }
        answer
    }

    /// Advisory name check with transport failures kept distinct from the
    /// authoritative "taken" answer. In particular, bounded IPC overload must
    /// not masquerade as `NameFree(false)` and leak into player-facing state as
    /// a false collision.
    async fn require_name_free(&self, char_name: &str) -> Result<(), CreateErr> {
        match self
            .request_past_commit(ServiceReq::NameFree {
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
    ///
    /// **16b cut F.4**: `account_id` is the account's immutable id, and it rides
    /// on the request. The server inserts `character -> account` into its live
    /// account oracle in the turn that creates the Player — before this
    /// function's own `link_character` writes the store document the next boot
    /// will re-derive the map from — so a character created after boot can use
    /// an account-scoped surface straight away. `None` only when the id could
    /// not be resolved at all; the character is then unmapped and fails closed.
    async fn create_and_link(
        &self,
        account: &str,
        account_id: Option<mush_core::AccountId>,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        if account_id.is_none() {
            tracing::error!(
                account,
                char_name,
                "creating a character for an account with no resolvable id; it will be unmapped \
                 until the next boot re-derives the account map"
            );
        }
        let id = match self
            .request_past_commit(ServiceReq::CreatePlayer {
                name: char_name.to_string(),
                account: account_id,
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
                // **16b cut F.4**: and the live oracle has to give the mapping
                // back. The server inserted `character -> account` in the create
                // turn against the document this write was going to make; with
                // the document missing, a RAM mapping that outlived it would
                // have the two halves disagree in the permissive direction until
                // the next boot re-derives the map.
                self.forget_character(account, id, char_name).await;
                Err(CreateErr::Io(e.to_string()))
            }
        }
    }

    /// Take a character back out of the server's live account oracle (16b cut
    /// F.4), after the account document it was inserted against could not be
    /// written. Best-effort and loud: a refusal leaves the mapping standing
    /// until the next boot, and an operator is told which character it is.
    async fn forget_character(&self, account: &str, id: ObjId, char_name: &str) {
        match self
            .link
            .request(ServiceReq::UnlinkPlayer { character: id })
            .await
        {
            Some(ServiceResp::UnlinkPlayer(_)) => {}
            other => tracing::error!(
                account,
                %id,
                char_name,
                answer = ?other,
                "the account oracle did not give the unlinked character back; it stays mapped in \
                 the running server until the next boot re-derives the map from the account \
                 documents"
            ),
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
        let created = unix_now_s();
        let account_id = mush_core::AccountId::mint(account, created).ok();
        let rec = AccountRec {
            // Mint the immutable id at creation from `(lowercased name,
            // created)` so a new account is mappable without a reboot; a mint
            // failure falls back to `None`, which the boot back-fill repairs.
            id: account_id,
            name: account.to_string(),
            argon2_hash: hash,
            created,
            characters: Vec::new(),
            ssh_keys: Vec::new(),
            discord: None,
        };
        match self.store.create_account(rec).await {
            Ok(()) => {}
            Err(StoreError::AccountExists) => return Err(CreateErr::AccountExists),
            Err(e) => return Err(CreateErr::Io(e.to_string())),
        }
        self.create_and_link(account, account_id, char_name).await
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
        self.create_and_link(account, account_id_of(&rec), char_name)
            .await
    }
}

/// **One account record's immutable id** (16b cut F.4): the stored one, else
/// minted from `(name, created)` — the same derivation
/// `mush_server::accounts_identity::hydrate` uses, so a character linked at run
/// time and the same character re-hydrated at the next boot resolve to one id.
fn account_id_of(rec: &AccountRec) -> Option<mush_core::AccountId> {
    rec.id
        .or_else(|| mush_core::AccountId::mint(&rec.name, rec.created).ok())
}

#[cfg(test)]
mod commit_retry_tests {
    //! **The creation flow waits out a commit** (16b cut F.4 follow-up).
    //!
    //! The lens for it is `crates/mush-gateway/tests/front_door_split.rs`, where
    //! the world is genuinely mid-commit on the layers it has just booted while
    //! the first `create` lands. These are the same thing in the small, with a
    //! link that answers a chosen number of `Busy`s first — and on tokio's paused
    //! clock, so the whole budget costs no wall clock.

    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use futures::future::BoxFuture;
    use mush_ipc::ToServer;
    use mush_store::InMemoryStore;

    /// A link that refuses the first `busies` requests as `Busy` and then
    /// answers. Counts every request it was sent.
    struct BusyLink {
        busies: usize,
        sent: AtomicUsize,
        then: ServiceResp,
    }

    impl BusyLink {
        fn new(busies: usize, then: ServiceResp) -> Arc<BusyLink> {
            Arc::new(BusyLink {
                busies,
                sent: AtomicUsize::new(0),
                then,
            })
        }
    }

    impl GatewayLink for BusyLink {
        fn to_server(&self, _msg: ToServer) -> BoxFuture<'_, bool> {
            Box::pin(async { false })
        }

        fn request(&self, _req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
            let nth = self.sent.fetch_add(1, Ordering::Relaxed);
            let answer = if nth < self.busies {
                ServiceResp::Busy
            } else {
                self.then.clone()
            };
            Box::pin(async move { Some(answer) })
        }
    }

    fn accounts_over(link: Arc<BusyLink>) -> Accounts {
        Accounts::with_link(Arc::new(InMemoryStore::new()), link, 3)
    }

    /// A name check that meets a commit gets its real answer, not a refusal the
    /// player would have to retype a whole `create` line for.
    #[tokio::test(start_paused = true)]
    async fn the_name_check_waits_for_the_commit_to_land() {
        let link = BusyLink::new(3, ServiceResp::NameFree(true));
        let accounts = accounts_over(Arc::clone(&link));

        assert_eq!(accounts.require_name_free("Rowan").await, Ok(()));
        assert_eq!(
            link.sent.load(Ordering::Relaxed),
            4,
            "three refusals were re-sent, and the fourth request was answered"
        );
    }

    /// The answers that are not the transient are still the first answer: no
    /// re-send, and each caller's mapping of them is untouched.
    #[tokio::test(start_paused = true)]
    async fn an_answered_check_is_not_re_sent() {
        for (answer, expected) in [
            (
                ServiceResp::NameFree(false),
                Err(CreateErr::CharacterNameTaken),
            ),
            (
                ServiceResp::Unavailable,
                Err(CreateErr::World(CreatePlayerError::WorldGone.to_string())),
            ),
        ] {
            let link = BusyLink::new(0, answer);
            let accounts = accounts_over(Arc::clone(&link));

            assert_eq!(accounts.require_name_free("Rowan").await, expected);
            assert_eq!(link.sent.load(Ordering::Relaxed), 1);
        }
    }

    /// A world that never finishes committing must not hold the welcome screen
    /// open: the budget ends, and the player gets the same busy refusal the flow
    /// reported before there was a budget.
    #[tokio::test(start_paused = true)]
    async fn a_world_that_stays_mid_commit_is_still_reported_busy() {
        let link = BusyLink::new(usize::MAX, ServiceResp::NameFree(true));
        let accounts = accounts_over(Arc::clone(&link));

        assert_eq!(
            accounts.require_name_free("Rowan").await,
            Err(CreateErr::World(
                "the game server is busy; please try again shortly".to_string()
            ))
        );
        assert_eq!(
            link.sent.load(Ordering::Relaxed),
            COMMIT_RETRY_ATTEMPTS,
            "the attempt count bounds the re-sends"
        );
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
                id: None,
                name: "Nick".into(),
                argon2_hash: "x".into(),
                created: unix_now_s(),
                characters: Vec::new(),
                ssh_keys: Vec::new(),
                discord: None,
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
