//! The account-distinctness oracle: hydrated at boot, kept current by the world
//! task (E7 cut 5 B1; 10b §3.5, OPEN 4, D-E7-34; 16b cut F.4).
//!
//! The world consults `mush_core::AccountIdentity` **synchronously inside a
//! command** (cut 5 B2's `+focus`, and the simulator's `SIM TEAM`), so it cannot
//! be a store scan. This module builds the character → account-id map once at
//! boot from `ObjectStore::list_accounts`, mints each account's id from its
//! `(lowercased name, created)` when the stored `id` is absent (deterministic;
//! the same `(name, created)` always yields the same id), and hands back an
//! `Arc<dyn AccountIdentity>` the composition installs into `mush-sim` **and**
//! into the engine — one instance behind both readers.
//!
//! # Why it is mutable (16b cut F.4, the recorded K6 point)
//!
//! The boot-built map was immutable, so a character created *after* boot was
//! unmapped until the next restart: two players who registered on playtest night
//! were told "your account is not mapped yet" and no advice could help them. The
//! map now learns. [`AccountIdentity::link`] is called by the world task in the
//! same turn that creates the Player, so the mapping exists before the create
//! reply leaves the world; [`AccountIdentity::unlink`] is called from the
//! destroy observer. The lock is a `std::sync::RwLock` because the readers are
//! synchronous and on the world task while the writer is the same task — the
//! lock exists to satisfy `&self` sharing, and is never held across an await.
//!
//! Fail-closed by construction: a character that is **not** in the map — never
//! linked, its account absent, or (the integrity case) **claimed by two
//! accounts** — resolves to `None`, and every calling policy refuses. A
//! character claimed by two accounts leaves **both** unmapped and is paged
//! (a structured error naming the accounts), never "first wins"; a `link` for a
//! character another account already holds is refused for the same reason. A
//! poisoned lock stops the map *learning* — both writers refuse and say so once
//! — while reads go on answering from the table it kept (`AccountMap::read`
//! says why).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use mush_core::account::{AccountIdentityEpoch, AccountPairSnapshot, PersonalAccountSnapshot};
use mush_core::{AccountId, AccountIdentity, ObjId};
use mush_store::ObjectStore;

/// Upper bound on characters carried in the boot-hydrated map. A server with
/// more mapped characters logs and maps only what fits (fail-closed for the
/// rest); this bounds what the boot pass *visits*, never refusing a live world.
pub const MAX_HYDRATED_ACCOUNTS: usize = 1_000_000;

/// The boot-hydrated map, kept current by the world task. `account_of` and the
/// derived predicates on [`AccountIdentity`] answer `None` for any character not
/// present.
#[derive(Debug, Default)]
pub struct AccountMap {
    by_character: RwLock<BTreeMap<ObjId, AccountId>>,
    /// Set the first time a poisoned lock is seen, so the operator is told
    /// once. A read runs inside a world command, so an unlatched line would
    /// make one panic into an error line per command for the rest of the
    /// process's life.
    poison_said: AtomicBool,
    epoch: AccountIdentityEpoch,
}

impl AccountMap {
    /// Build a map over an already-resolved table (the boot pass's second pass,
    /// and the unit tests).
    #[must_use]
    pub fn from_map(by_character: BTreeMap<ObjId, AccountId>) -> AccountMap {
        AccountMap {
            by_character: RwLock::new(by_character),
            poison_said: AtomicBool::new(false),
            epoch: AccountIdentityEpoch::default(),
        }
    }

    /// How many characters are mapped right now.
    #[must_use]
    pub fn len(&self) -> usize {
        self.read().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// The table, **including** when the lock is poisoned.
    ///
    /// A poisoned lock means a panic while the map was held. Nothing this module
    /// does under the guard can panic — every one is a single infallible
    /// `BTreeMap` call — so the table a poisoned lock hands back is intact, and
    /// reads go on answering from it. What stops is *learning*: [`Self::link`]
    /// and [`Self::unlink`] refuse while the lock is poisoned, so nothing new is
    /// recorded into a map whose integrity is in doubt and every character
    /// created after the panic fails closed like an unmapped one. The world
    /// task's firewall turns the panic itself into an orderly exit.
    ///
    /// Returning an empty table instead would be the *other* posture, and a
    /// worse one here: it would quietly unmap every character on the server —
    /// `+focus`, `SIM TEAM` and the Discord relay's `(account, actor)` re-check
    /// all fail closed — on a lock that has lost nothing.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<ObjId, AccountId>> {
        self.by_character.read().unwrap_or_else(|poisoned| {
            self.say_poisoned("reads continue over the live table; nothing more is learned");
            poisoned.into_inner()
        })
    }

    /// Say a poisoned lock ONCE, whichever of the three paths meets it first.
    fn say_poisoned(&self, consequence: &str) {
        self.epoch.invalidate();
        if !self.poison_said.swap(true, Ordering::Relaxed) {
            tracing::error!(
                consequence,
                "account-identity: the map's lock is poisoned by an earlier panic"
            );
        }
    }
}

impl AccountIdentity for AccountMap {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.read().get(&character).copied()
    }

    fn capture_personal_account(&self, character: ObjId) -> Option<PersonalAccountSnapshot> {
        let table = match self.by_character.read() {
            Ok(table) => table,
            Err(_) => {
                self.say_poisoned("composed account observations are unavailable");
                return None;
            }
        };
        self.epoch
            .personal_account(character, *table.get(&character)?)
    }

    fn observe_distinct_accounts(
        &self,
        requester: ObjId,
        other: ObjId,
    ) -> Option<AccountPairSnapshot> {
        let table = match self.by_character.read() {
            Ok(table) => table,
            Err(_) => {
                self.say_poisoned("composed account observations are unavailable");
                return None;
            }
        };
        self.epoch.distinct_pair(
            requester,
            *table.get(&requester)?,
            other,
            *table.get(&other)?,
        )
    }

    fn link(&self, character: ObjId, account: AccountId) -> bool {
        let Ok(mut table) = self.by_character.write() else {
            self.say_poisoned("a character was left unmapped and fails closed");
            tracing::error!(
                character = character.0,
                "account-identity: a link was refused by a poisoned lock; the character is left \
                 unmapped"
            );
            return false;
        };
        match table.get(&character) {
            // Idempotent: the same account twice is the create path retried.
            Some(held) if *held == account => true,
            // The integrity case, in RAM this time: two accounts claiming one
            // character. Leave what is there, refuse, and page — the same rule
            // `hydrate` applies to the store.
            Some(_) => {
                tracing::error!(
                    character = character.0,
                    "account-identity: a link named a character another account already holds; \
                     refused (fail closed). Resolve the account documents before retrying."
                );
                false
            }
            None => {
                self.epoch.invalidate();
                table.insert(character, account);
                true
            }
        }
    }

    fn unlink(&self, character: ObjId) -> bool {
        let Ok(mut table) = self.by_character.write() else {
            self.say_poisoned("a character was left mapped");
            tracing::error!(
                character = character.0,
                "account-identity: an unlink was refused by a poisoned lock; the character is \
                 left mapped"
            );
            return false;
        };
        if table.contains_key(&character) {
            self.epoch.invalidate();
            table.remove(&character);
            true
        } else {
            false
        }
    }
}

/// Build the oracle from the store, minting missing ids on the fly. Bounded by
/// [`MAX_HYDRATED_ACCOUNTS`]; a character claimed by two accounts leaves both
/// unmapped and pages. Returns an `Arc<dyn AccountIdentity>` for
/// `SimulationModule::with_accounts`.
pub async fn hydrate(store: &dyn ObjectStore) -> Result<AccountMap, String> {
    let accounts = store
        .list_accounts()
        .await
        .map_err(|error| format!("account-identity hydration: {error}"))?;

    // Pass 1: collect, per character, the set of account ids that claim it.
    // A character in one account maps; a character in two is a data-integrity
    // fault and both must be unmapped (never first-wins).
    let mut claims: BTreeMap<ObjId, Vec<(AccountId, String)>> = BTreeMap::new();
    let mut visited = 0usize;
    let mut truncated = false;
    for rec in &accounts {
        // The immutable id: the stored one, else minted from (name, created).
        // Deterministic, so a legacy None-id account still maps.
        let Some(id) = rec
            .id
            .or_else(|| AccountId::mint(&rec.name, rec.created).ok())
        else {
            tracing::error!(
                account = %rec.name,
                "account-identity: id could not be minted; account left unmapped"
            );
            continue;
        };
        for character in &rec.characters {
            if visited >= MAX_HYDRATED_ACCOUNTS {
                truncated = true;
                break;
            }
            visited += 1;
            claims
                .entry(character.id)
                .or_default()
                .push((id, rec.name.clone()));
        }
        if truncated {
            break;
        }
    }
    if truncated {
        tracing::error!(
            cap = MAX_HYDRATED_ACCOUNTS,
            accounts = accounts.len(),
            "account-identity: hydration hit the character cap; the remainder is unmapped \
             and fails closed — reduce linked characters or raise MAX_HYDRATED_ACCOUNTS"
        );
    }

    // Pass 2: map only the characters exactly one account claims.
    let mut by_character = BTreeMap::new();
    for (character, owners) in claims {
        match owners.as_slice() {
            [(id, _)] => {
                by_character.insert(character, *id);
            }
            many => {
                // Page: a genuine data-integrity decision, with the accounts
                // named and the resolution (unlink the character from all but
                // one). Both stay unmapped; the oracle answers None; +focus
                // refuses. Never stop the world for a local problem.
                let names: Vec<&str> = many.iter().map(|(_, name)| name.as_str()).collect();
                tracing::error!(
                    character = character.0,
                    accounts = ?names,
                    "account-identity: character claimed by multiple accounts; left unmapped \
                     (fail closed). Resolve with @unlink so exactly one account holds it."
                );
            }
        }
    }

    Ok(AccountMap::from_map(by_character))
}

/// The unwired oracle for a composition with no store-backed accounts (tests,
/// and the default composition). Answers `None` to everything.
#[must_use]
pub fn empty_account_identity() -> Arc<dyn AccountIdentity> {
    Arc::new(AccountMap::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_store::{AccountRec, CharacterRef, InMemoryStore};

    fn account(name: &str, created: i64, chars: &[(u32, &str)]) -> AccountRec {
        AccountRec {
            id: None,
            name: name.to_string(),
            argon2_hash: "$argon2id$fake".to_string(),
            created,
            characters: chars
                .iter()
                .map(|(id, n)| CharacterRef {
                    id: ObjId(*id),
                    name: (*n).to_string(),
                })
                .collect(),
            ssh_keys: vec![],
            discord: None,
        }
    }

    #[tokio::test]
    async fn e7c5_b1_hydrate_maps_characters_to_their_account() {
        let store = InMemoryStore::new();
        store
            .create_account(account("Nick", 1, &[(7, "Nicky"), (8, "Alt")]))
            .await
            .unwrap();
        store
            .create_account(account("Zoe", 2, &[(9, "Zed")]))
            .await
            .unwrap();
        let map = hydrate(&store).await.unwrap();
        // Nicky and Alt share Nick's account; Zed is a different account.
        assert_eq!(map.account_of(ObjId(7)), map.account_of(ObjId(8)));
        assert_ne!(map.account_of(ObjId(7)), map.account_of(ObjId(9)));
        assert_eq!(map.same_account(ObjId(7), ObjId(8)), Some(true));
        assert_eq!(map.same_account(ObjId(7), ObjId(9)), Some(false));
        // An unlinked character fails closed.
        assert_eq!(map.account_of(ObjId(99)), None);
        assert_eq!(map.same_account(ObjId(7), ObjId(99)), None);
    }

    #[tokio::test]
    async fn e7c5_b1_a_character_in_two_accounts_leaves_both_unmapped() {
        let store = InMemoryStore::new();
        // ObjId(7) is (wrongly) linked to both accounts.
        store
            .create_account(account("Nick", 1, &[(7, "Nicky")]))
            .await
            .unwrap();
        store
            .create_account(account("Zoe", 2, &[(7, "Stolen"), (9, "Zed")]))
            .await
            .unwrap();
        let map = hydrate(&store).await.unwrap();
        // The duplicated character is unmapped; the clean character still maps.
        assert_eq!(
            map.account_of(ObjId(7)),
            None,
            "duplicated character is unmapped"
        );
        assert!(
            map.account_of(ObjId(9)).is_some(),
            "the clean character still maps"
        );
        // The oracle refuses every question about the duplicated character.
        assert_eq!(map.same_account(ObjId(7), ObjId(9)), None);
    }

    /// **16b cut F.4, the K6 point**: a character the map learns of after boot
    /// answers like one hydration found, and one it forgets fails closed again.
    #[tokio::test]
    async fn f4_a_character_linked_after_boot_maps_and_unlinking_forgets_it() {
        let store = InMemoryStore::new();
        store
            .create_account(account("Nick", 1, &[(7, "Nicky")]))
            .await
            .unwrap();
        let map = hydrate(&store).await.unwrap();
        let nick = map.account_of(ObjId(7)).expect("hydration mapped Nicky");
        assert_eq!(map.len(), 1);
        // Before the link, the new character fails closed.
        assert_eq!(map.account_of(ObjId(42)), None);
        // The world task's insert. It is the SAME account, so the alt and the
        // first character are one account to every policy that asks.
        assert!(map.link(ObjId(42), nick));
        assert_eq!(map.account_of(ObjId(42)), Some(nick));
        assert_eq!(map.same_account(ObjId(7), ObjId(42)), Some(true));
        assert_eq!(map.len(), 2);
        // Idempotent: the same pair twice is a retried create, not a fault.
        assert!(map.link(ObjId(42), nick));
        assert_eq!(map.len(), 2);
        // The destroy observer's half.
        assert!(map.unlink(ObjId(42)));
        assert_eq!(map.account_of(ObjId(42)), None);
        assert!(!map.unlink(ObjId(42)), "forgetting twice removes nothing");
        assert_eq!(map.len(), 1);
    }

    /// A second account claiming a character the map already holds is the
    /// integrity case `hydrate` refuses in the store, refused in RAM too: what
    /// is there stands and the link answers `false`.
    #[tokio::test]
    async fn f4_a_link_never_moves_a_character_to_another_account() {
        let store = InMemoryStore::new();
        store
            .create_account(account("Nick", 1, &[(7, "Nicky")]))
            .await
            .unwrap();
        store.create_account(account("Zoe", 2, &[])).await.unwrap();
        let map = hydrate(&store).await.unwrap();
        let nick = map.account_of(ObjId(7)).expect("Nicky maps");
        let zoe = AccountId::mint("Zoe", 2).unwrap();
        assert!(!map.link(ObjId(7), zoe), "the second claim is refused");
        assert_eq!(map.account_of(ObjId(7)), Some(nick), "the first stands");
    }

    /// **A poisoned lock stops the map learning and nothing else.** The posture
    /// the module head and [`AccountMap::read`] describe, pinned: the table the
    /// panic left behind still answers, and both writers refuse.
    #[tokio::test]
    async fn f4_a_poisoned_lock_stops_the_learning_and_keeps_the_answers() {
        let store = InMemoryStore::new();
        store
            .create_account(account("Nick", 1, &[(7, "Nicky")]))
            .await
            .unwrap();
        let map = Arc::new(hydrate(&store).await.unwrap());
        let nick = map.account_of(ObjId(7)).expect("hydration mapped Nicky");

        // Poison it the only way a lock is poisoned: panic while it is held.
        let holder = Arc::clone(&map);
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let _ = std::thread::spawn(move || {
            let _guard = holder.by_character.write().expect("a fresh lock");
            panic!("the world task fell over with the map in hand");
        })
        .join();
        std::panic::set_hook(hook);
        assert!(map.by_character.read().is_err(), "the lock is poisoned");

        // Reads answer from the table the panic left, which lost nothing.
        assert_eq!(map.account_of(ObjId(7)), Some(nick));
        assert_eq!(map.len(), 1);
        assert_eq!(map.account_of(ObjId(42)), None);
        // Neither writer takes anything, so a character made after the panic
        // fails closed exactly like one that was never linked.
        assert!(!map.link(ObjId(42), nick), "the link is refused");
        assert_eq!(map.account_of(ObjId(42)), None);
        assert!(!map.unlink(ObjId(7)), "the unlink is refused");
        assert_eq!(map.account_of(ObjId(7)), Some(nick), "and nothing moved");
    }

    /// The default trait bodies: an oracle that cannot learn says so, rather
    /// than accepting a write it would drop.
    #[test]
    fn f4_a_fixed_oracle_refuses_to_learn() {
        struct Fixed;
        impl AccountIdentity for Fixed {
            fn account_of(&self, _character: ObjId) -> Option<AccountId> {
                None
            }
        }
        let fixed = Fixed;
        assert!(fixed.capture_personal_account(ObjId(1)).is_none());
        assert!(!fixed.link(ObjId(1), AccountId::mint("Nick", 1).unwrap()));
        assert!(!fixed.unlink(ObjId(1)));
        assert!(
            fixed
                .observe_distinct_accounts(ObjId(1), ObjId(2))
                .is_none()
        );
    }

    #[tokio::test]
    async fn m7_account_capture_uses_hydrated_and_live_account_mapping() {
        let store = InMemoryStore::new();
        store
            .create_account(account("Nick", 1, &[(7, "Nicky"), (8, "Alt")]))
            .await
            .unwrap();
        store
            .create_account(account("Zoe", 2, &[(9, "Zed")]))
            .await
            .unwrap();
        let map = Arc::new(hydrate(&store).await.unwrap());
        for character in [7, 8, 9] {
            assert!(map.capture_personal_account(ObjId(character)).is_some());
        }
        assert!(map.capture_personal_account(ObjId(99)).is_none());
        assert!(map.observe_distinct_accounts(ObjId(7), ObjId(9)).is_some());
        assert!(map.observe_distinct_accounts(ObjId(9), ObjId(7)).is_some());
        for (left, right) in [(7, 7), (7, 8), (7, 99), (99, 9)] {
            assert!(
                map.observe_distinct_accounts(ObjId(left), ObjId(right))
                    .is_none()
            );
        }

        // The simulator holds this same oracle, not the Engine wrapper. Its
        // destruction observer must make a later account capture unavailable.
        let simulator_oracle: Arc<dyn AccountIdentity> = map.clone();
        assert!(simulator_oracle.unlink(ObjId(9)));
        assert!(map.capture_personal_account(ObjId(9)).is_none());
        assert!(map.observe_distinct_accounts(ObjId(7), ObjId(9)).is_none());
        let nick = map.account_of(ObjId(7)).unwrap();
        assert!(simulator_oracle.link(ObjId(9), nick));
        // Single-account capture deliberately does not require a second,
        // distinct account; known alternate characters are valid requesters.
        assert!(map.capture_personal_account(ObjId(9)).is_some());
        assert!(map.observe_distinct_accounts(ObjId(7), ObjId(9)).is_none());
    }

    #[test]
    fn m7_account_capture_refuses_poisoned_mapping_while_legacy_reads_continue() {
        let nick = AccountId::mint("Nick", 1).unwrap();
        let zoe = AccountId::mint("Zoe", 2).unwrap();
        let map = Arc::new(AccountMap::from_map(BTreeMap::from([
            (ObjId(7), nick),
            (ObjId(9), zoe),
        ])));
        assert!(map.observe_distinct_accounts(ObjId(7), ObjId(9)).is_some());
        assert!(map.capture_personal_account(ObjId(7)).is_some());
        let holder = Arc::clone(&map);
        assert!(
            std::thread::spawn(move || {
                let _guard = holder.by_character.write().unwrap();
                panic!("poison the account map");
            })
            .join()
            .is_err()
        );
        assert!(map.observe_distinct_accounts(ObjId(7), ObjId(9)).is_none());
        assert!(map.capture_personal_account(ObjId(7)).is_none());
        assert_eq!(map.account_of(ObjId(7)), Some(nick));
        assert_eq!(map.account_of(ObjId(9)), Some(zoe));
    }
}
