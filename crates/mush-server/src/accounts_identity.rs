//! The boot-hydrated account-distinctness oracle (E7 cut 5 B1; 10b §3.5,
//! OPEN 4, D-E7-34).
//!
//! The world consults `mush_core::AccountIdentity` **synchronously inside a
//! command** (cut 5 B2's `+focus`), so it cannot be a store scan. This module
//! builds the character → account-id map once at boot from
//! `ObjectStore::list_accounts`, mints each account's id from its
//! `(lowercased name, created)` when the stored `id` is absent (deterministic;
//! the same `(name, created)` always yields the same id), and hands back an
//! `Arc<dyn AccountIdentity>` the composition installs into `mush-sim`.
//!
//! Fail-closed by construction: a character that is **not** in the map — never
//! linked, its account absent, or (the integrity case) **claimed by two
//! accounts** — resolves to `None`, and every calling policy refuses. A
//! character claimed by two accounts leaves **both** unmapped and is paged
//! (a structured error naming the accounts), never "first wins".

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{AccountId, AccountIdentity, ObjId};
use mush_store::ObjectStore;

/// Upper bound on characters carried in the boot-hydrated map. A server with
/// more mapped characters logs and maps only what fits (fail-closed for the
/// rest); this bounds what the boot pass *visits*, never refusing a live world.
pub const MAX_HYDRATED_ACCOUNTS: usize = 1_000_000;

/// The immutable boot-hydrated map. `account_of` and the derived predicates on
/// [`AccountIdentity`] answer `None` for any character not present.
#[derive(Debug, Default)]
pub struct AccountMap {
    by_character: BTreeMap<ObjId, AccountId>,
}

impl AccountMap {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_character.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_character.is_empty()
    }
}

impl AccountIdentity for AccountMap {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.by_character.get(&character).copied()
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

    Ok(AccountMap { by_character })
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
}
