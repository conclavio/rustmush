//! Party boot: hydrate `party:{id}` through the wire, floor the allocator, re-derive
//! the projection, and drop — loudly — anything that will not load (E6a).
//!
//! ## The document discipline
//!
//! | doc | a corrupt one |
//! |---|---|
//! | `party:{id}` | dropped loudly, and a `Del` fixup queued so the store matches RAM |
//! | `meta` | tolerated field by field, and floored by the keys either way |
//!
//! A `party:{id}` is a container document — its state *is* the record — and it still
//! does not fail the boot, which is the one place this crate departs from
//! `mush-housing`'s container rule on purpose. A tenancy is somebody's **home**:
//! refusing to boot is the safe answer, because booting without it opens a door.
//! A party is **ephemeral by construction** (§3.1: it expires after
//! `PARTY_IDLE_TTL` of silence, and nothing outside it holds a reference — D-E6-5
//! forbids the contract pointing back), so a document nobody can read costs the
//! players one `+party/create`, and taking the whole world down for it would be the
//! larger failure. The drop is therefore leaf-shaped: `tracing::warn!` naming the
//! key and the reason, plus a `Del`.
//!
//! **What is dropped is never the check.** `Party` deserializes only through its
//! `TryFrom<PartyWire>`, so every §9 cap and every §3.1 invariant runs on the way in
//! (the E5a fold rule); a nine-member roster, a role filed for somebody off the
//! roster, a duplicated member and invitations on a disbanded party all fail *here*,
//! at load, and none of them reaches RAM.
//!
//! ## Identity and the allocator
//!
//! - **DL-07 identity**: a document whose `id` disagrees with its key's id is dropped
//!   with a loud skip. `obj == key` or it does not load.
//! - **The floor is taken from the key** whether or not the document parses, and
//!   `saturating`: an id that was minted was minted, and re-minting it would attach a
//!   fresh party to a key somebody's dropped document still occupies. A hand-written
//!   `party:18446744073709551615` parks the allocator at the ceiling, where
//!   [`PartyStore::alloc_party`] refuses, rather than wrapping.
//! - `next_party` = `max(persisted next_party_id, max loaded key id + 1)`, saturating.

use mush_core::module::{ModError, ModuleBootData, ModuleCtx};
use smol_str::SmolStr;

use crate::ids::PartyId;
use crate::module::{META_KEY, PartyStore, drop_doc, persist_meta};
use crate::record::Party;

/// What one boot decided about the persisted documents.
struct Loaded {
    store: PartyStore,
    /// Keys whose documents did not load — a `Del` is queued for each so the store
    /// matches RAM.
    dropped: Vec<SmolStr>,
}

/// Parse the persisted documents into a fresh store. Never fails: everything it
/// cannot read, it drops loudly.
fn parse(data: &ModuleBootData) -> Loaded {
    let mut store = PartyStore::default();
    let mut dropped: Vec<SmolStr> = Vec::new();
    let mut meta_next: u64 = 0;
    let mut max_key: Option<u64> = None;

    for (key, value) in &data.kv {
        if key == META_KEY {
            meta_next = value
                .get("next_party_id")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            continue;
        }
        let Some(digits) = key.strip_prefix("party:") else {
            tracing::warn!(%key, "party: unrecognised persisted key; dropping");
            dropped.push(key.clone());
            continue;
        };
        let Ok(id) = digits.parse::<u64>() else {
            tracing::warn!(%key, "party: bad party key; dropping");
            dropped.push(key.clone());
            continue;
        };
        // The floor is taken from the key even when the document does not load.
        max_key = Some(max_key.map_or(id, |m: u64| m.max(id)));
        match serde_json::from_value::<Party>(value.clone()) {
            Ok(p) if p.id == PartyId(id) => {
                store.parties.insert(p.id, p);
            }
            // DL-07: `obj == key`, or a loud skip.
            Ok(p) => {
                tracing::warn!(%key, claims = %p.id, "party: doc id disagrees with its key; dropping");
                dropped.push(key.clone());
            }
            Err(e) => {
                tracing::warn!(%key, error = %e, "party: doc failed its wire; dropping");
                dropped.push(key.clone());
            }
        }
    }

    store.next_party = match max_key {
        Some(m) => meta_next.max(m.saturating_add(1)),
        None => meta_next,
    };
    store.reindex_live();
    Loaded { store, dropped }
}

/// Hydrate the store from `data` and queue the fixups.
///
/// # Errors
///
/// [`ModError`] only when the state slot is missing — a registration bug, not a data
/// one. No document can fail this boot.
pub(crate) fn load(ctx: &mut ModuleCtx<'_>, data: &ModuleBootData) -> Result<(), ModError> {
    let Loaded { store, dropped } = parse(data);
    {
        let slot = ctx
            .w
            .module_state_mut::<PartyStore>()
            .ok_or_else(|| ModError::msg("party: state slot missing"))?;
        *slot = store;
    }
    for key in dropped {
        drop_doc(ctx, key);
    }
    // The floor may have moved past what `meta` said; write it back so the next boot
    // starts from the same place even if the keys are gone.
    persist_meta(ctx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::ids::PartyId;
    use crate::record::Party;
    use mush_core::ObjId;
    use mush_core::module::ModuleBootData;
    use smol_str::SmolStr;
    use std::collections::HashMap;

    fn boot(kv: Vec<(SmolStr, serde_json::Value)>) -> ModuleBootData {
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        }
    }

    fn doc(id: u64) -> serde_json::Value {
        let p = Party::new(PartyId(id), "Crew", ObjId(10), 0).expect("valid");
        serde_json::to_value(&p).expect("serializes")
    }

    #[test]
    fn the_allocator_floors_on_keys_and_meta_saturating() {
        let loaded = parse(&boot(vec![
            (
                SmolStr::new("meta"),
                serde_json::json!({"next_party_id": 2}),
            ),
            (SmolStr::new("party:9"), doc(9)),
        ]));
        assert_eq!(loaded.store.next_party, 10, "the key floors past the meta");

        let loaded = parse(&boot(vec![
            (
                SmolStr::new("meta"),
                serde_json::json!({"next_party_id": 40}),
            ),
            (SmolStr::new("party:9"), doc(9)),
        ]));
        assert_eq!(loaded.store.next_party, 40, "the meta floors past the key");

        let mut store = parse(&boot(vec![(
            SmolStr::new(format!("party:{}", u64::MAX)),
            doc(u64::MAX),
        )]))
        .store;
        assert_eq!(store.next_party, u64::MAX, "saturating, never wrapping");
        assert_eq!(
            store.alloc_party(),
            None,
            "parked at the ceiling, minting refuses rather than re-issuing a live id"
        );
    }

    #[test]
    fn a_bad_key_and_a_bad_doc_are_dropped_and_the_floor_survives() {
        let mut bad = doc(3);
        bad["members"] = serde_json::json!([10, 10]);
        let loaded = parse(&boot(vec![
            (SmolStr::new("party:oops"), doc(1)),
            (SmolStr::new("party:3"), bad),
            (SmolStr::new("rubbish"), serde_json::json!({})),
            (SmolStr::new("party:4"), doc(4)),
        ]));
        assert_eq!(loaded.store.parties.len(), 1);
        assert!(loaded.store.party(PartyId(4)).is_some());
        assert_eq!(loaded.store.next_party, 5, "party:3's key still floored it");
        assert_eq!(loaded.dropped.len(), 3);
    }

    #[test]
    fn an_id_key_mismatch_is_dropped() {
        let loaded = parse(&boot(vec![(SmolStr::new("party:7"), doc(8))]));
        assert!(loaded.store.parties.is_empty());
        assert_eq!(loaded.dropped, vec![SmolStr::new("party:7")]);
        assert_eq!(
            loaded.store.next_party, 8,
            "the key floored it, not the doc"
        );
    }

    #[test]
    fn the_live_projection_is_rederived_from_the_status() {
        let mut disbanded = Party::new(PartyId(5), "Gone", ObjId(10), 0).expect("valid");
        disbanded.disband(crate::record::DisbandCause::Expired);
        let loaded = parse(&boot(vec![
            (SmolStr::new("party:4"), doc(4)),
            (
                SmolStr::new("party:5"),
                serde_json::to_value(&disbanded).expect("serializes"),
            ),
        ]));
        assert_eq!(loaded.store.live_count(), 1);
        assert_eq!(
            loaded.store.live_ids().collect::<Vec<_>>(),
            vec![PartyId(4)]
        );
    }
}
