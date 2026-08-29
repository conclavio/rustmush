//! NPC **world / semantic knowledge** (M10d) — what an NPC knows by virtue of
//! *who and where it is*, independent of any conversation. This is the second
//! kind of NPC knowledge, distinct from the episodic
//! [`memory`](crate::memory) stack (what it remembers experiencing):
//!
//! - a bartender knows the local streets; an occult bookseller knows about
//!   vampires — facts that hold whether or not the NPC has ever met you.
//!
//! World knowledge has two sources: per-NPC [`LoreEntry`]s authored on the NPC
//! (stored as JSON under [`KNOWLEDGE_ATTR`], like the
//! [`AgentProfile`](crate::profile::AgentProfile) under
//! [`PROFILE_ATTR`](crate::profile::PROFILE_ATTR)), and shared [`LorePool`]s
//! authored once and referenced by many NPCs via tags.
//!
//! Each entry carries a [`Secrecy`] tier that governs whether it may be shared
//! and, crucially, whether it may ever reach the LLM at all:
//!
//! - [`Secrecy::Public`] — shared freely.
//! - [`Secrecy::Guarded`] — known but held back; revealed only to people the
//!   NPC trusts.
//! - [`Secrecy::Secret`] — plot-critical; a **staff-only** record that is
//!   **never** placed in any prompt or snapshot. A `Secret` fact reaches the
//!   game only through scripted / staff events, never through the NPC's own
//!   deliberation.
//!
//! Willingness to share is `f(secrecy tier, trust in the asker, NPC nature)`.
//! In this first cut trust is a [`Familiarity`] level *derived from episodic
//! memory* ([`familiarity_of`]): does the NPC already remember this player?
//! Staff-set disposition is a documented future iteration.

use std::collections::HashMap;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::memory::{self, LongTermStore};

/// The attribute an NPC's [`NpcKnowledge`] is stored under (JSON), in the same
/// `agent` namespace as [`PROFILE_ATTR`](crate::profile::PROFILE_ATTR).
pub const KNOWLEDGE_ATTR: &str = "agent:knowledge";

/// The secrecy tier of one world-knowledge [`LoreEntry`] — how freely the NPC
/// may share it, and (for [`Secrecy::Secret`]) whether it may ever enter a
/// prompt at all.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Secrecy {
    /// Shared freely with anyone.
    #[default]
    Public,
    /// Known but held back — shared only with those the NPC trusts.
    Guarded,
    /// Plot-critical: a staff-only record **never** placed in any prompt or
    /// snapshot. Acts on the game only via scripted / staff events.
    Secret,
}

/// One world-knowledge fact the NPC knows by virtue of who/where it is
/// (design: M10d). Stored per-NPC under [`KNOWLEDGE_ATTR`] and inside shared
/// [`LorePool`]s. Deserialization is lenient (`#[serde(default)]`) so sparse or
/// older documents always load.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoreEntry {
    /// Who/what the fact is about (a name or topic). May be `""` (empty).
    pub subject: String,
    /// The fact itself.
    pub fact: String,
    /// The secrecy tier governing sharing (and prompt eligibility).
    pub secrecy: Secrecy,
    /// Provenance: how/why the NPC knows this. May be `""` (empty).
    pub source: String,
}

impl LoreEntry {
    /// Whether this entry may ever be placed in a prompt or snapshot — true for
    /// [`Secrecy::Public`] and [`Secrecy::Guarded`], **false** for
    /// [`Secrecy::Secret`] (which is never sent to the NPC LLM).
    ///
    /// Written as a positive **allowlist** (not `!= Secret`) so it fails safe:
    /// if a future, more-restrictive tier is ever added it stays out of prompts
    /// until explicitly allowed here — matching the allowlist polarity the
    /// prompt renderer (`render_world_knowledge`) already uses, so both
    /// defense-in-depth layers behave identically.
    #[must_use]
    pub fn is_shareable(&self) -> bool {
        matches!(self.secrecy, Secrecy::Public | Secrecy::Guarded)
    }
}

/// The per-NPC world-knowledge document, stored as JSON under
/// [`KNOWLEDGE_ATTR`] on the NPC object. Lenient deserialize so sparse / older
/// docs load.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NpcKnowledge {
    /// Facts authored directly on this NPC.
    pub lore: Vec<LoreEntry>,
    /// Shared-lore pool tags this NPC draws on (resolved via [`LorePools`]).
    pub tags: Vec<String>,
}

/// A shared lore pool — a set of [`LoreEntry`]s authored once and referenced by
/// many NPCs via its `tag`. Persisted as the module document keyed by
/// [`pool_key`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LorePool {
    /// The tag NPCs reference this pool by.
    pub tag: String,
    /// The pool's entries.
    pub entries: Vec<LoreEntry>,
}

/// The persistence key one [`LorePool`] lives under (tag slugged so it is a safe
/// key), mirroring [`record_key`](crate::memory::record_key)'s shape.
#[must_use]
pub fn pool_key(tag: &str) -> SmolStr {
    SmolStr::new(format!("lore:{}", memory::slug(tag)))
}

/// The in-RAM view of every shared [`LorePool`] on the server, keyed by
/// `slug(tag)`. Held in [`NpcHostState`](crate::runtime::NpcHostState),
/// rebuilt from boot data on load and mirrored by the persisted documents.
#[derive(Clone, Debug, Default)]
pub struct LorePools {
    pools: HashMap<String, LorePool>,
}

impl LorePools {
    /// Load one pool (from boot data), keyed by its slugged tag.
    pub fn insert(&mut self, pool: LorePool) {
        self.pools.insert(memory::slug(&pool.tag), pool);
    }

    /// Insert or replace the pool for its tag (staff edit), keeping the in-RAM
    /// view in step with a persisted write.
    pub fn set(&mut self, pool: LorePool) {
        self.insert(pool);
    }

    /// The pool for `tag`, if any.
    #[must_use]
    pub fn get(&self, tag: &str) -> Option<&LorePool> {
        self.pools.get(&memory::slug(tag))
    }

    /// Remove the pool for `tag`. Returns whether one was present.
    pub fn remove(&mut self, tag: &str) -> bool {
        self.pools.remove(&memory::slug(tag)).is_some()
    }

    /// The concatenated entries of every pool matching `tags`, in a
    /// deterministic order (the given tag order, then each pool's entry order).
    /// Unknown tags are ignored.
    #[must_use]
    pub fn resolve(&self, tags: &[String]) -> Vec<LoreEntry> {
        let mut out = Vec::new();
        for tag in tags {
            if let Some(pool) = self.pools.get(&memory::slug(tag)) {
                out.extend(pool.entries.iter().cloned());
            }
        }
        out
    }

    /// The number of pools loaded.
    #[must_use]
    pub fn tag_count(&self) -> usize {
        self.pools.len()
    }

    /// Whether no pools are loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }

    /// Every pool tag, sorted (for staff listing).
    #[must_use]
    pub fn tags_sorted(&self) -> Vec<String> {
        let mut v: Vec<String> = self.pools.values().map(|p| p.tag.clone()).collect();
        v.sort();
        v
    }
}

/// How well an NPC knows a particular player, derived from episodic memory
/// (design: M10d first cut — no staff-set disposition yet). Governs whether a
/// [`Secrecy::Guarded`] fact may be shared.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Familiarity {
    /// No durable memory of this player at all.
    #[default]
    Stranger,
    /// The NPC has some durable memory of this player.
    Acquaintance,
    /// The NPC has a well-established memory of this player (>= 3 notes).
    Known,
}

/// The number of durable notes at which an [`Acquaintance`](Familiarity::Acquaintance)
/// becomes [`Known`](Familiarity::Known).
const KNOWN_NOTES: usize = 3;

/// The NPC's [`Familiarity`] with `player_name`, derived from its durable
/// memory in `store`: [`Stranger`](Familiarity::Stranger) if there is no record,
/// [`Acquaintance`](Familiarity::Acquaintance) if there is one, and
/// [`Known`](Familiarity::Known) once that record holds at least three notes.
#[must_use]
pub fn familiarity_of(store: &LongTermStore, npc: ObjId, player_name: &str) -> Familiarity {
    match store.record_for(npc, player_name) {
        None => Familiarity::Stranger,
        Some(rec) if rec.notes.len() >= KNOWN_NOTES => Familiarity::Known,
        Some(_) => Familiarity::Acquaintance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrecy_default_is_public() {
        assert_eq!(Secrecy::default(), Secrecy::Public);
    }

    #[test]
    fn is_shareable_gates_only_secret() {
        let public = LoreEntry {
            fact: "the ale is watered".into(),
            secrecy: Secrecy::Public,
            ..LoreEntry::default()
        };
        let guarded = LoreEntry {
            fact: "the sheriff takes bribes".into(),
            secrecy: Secrecy::Guarded,
            ..LoreEntry::default()
        };
        let secret = LoreEntry {
            fact: "the vault code is 1234".into(),
            secrecy: Secrecy::Secret,
            ..LoreEntry::default()
        };
        assert!(public.is_shareable());
        assert!(guarded.is_shareable());
        assert!(!secret.is_shareable());
    }

    #[test]
    fn sparse_lore_entry_deserializes_with_defaults() {
        let e: LoreEntry = serde_json::from_str(r#"{"fact":"knows the docks"}"#).unwrap();
        assert_eq!(e.fact, "knows the docks");
        assert_eq!(e.subject, "");
        assert_eq!(e.source, "");
        assert_eq!(e.secrecy, Secrecy::Public);
    }

    fn entry(fact: &str) -> LoreEntry {
        LoreEntry {
            fact: fact.into(),
            ..LoreEntry::default()
        }
    }

    #[test]
    fn pools_resolve_unions_matching_and_ignores_unknown() {
        let mut pools = LorePools::default();
        pools.insert(LorePool {
            tag: "Docks".into(),
            entries: vec![entry("smugglers land at midnight")],
        });
        pools.insert(LorePool {
            tag: "occult".into(),
            entries: vec![entry("vampires shun sunlight"), entry("silver burns lupines")],
        });

        // Tag matching is slug-insensitive; unknown tags contribute nothing.
        let resolved = pools.resolve(&[
            "docks".into(),
            "occult".into(),
            "no-such-pool".into(),
        ]);
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].fact, "smugglers land at midnight");
        assert_eq!(resolved[1].fact, "vampires shun sunlight");
        assert_eq!(resolved[2].fact, "silver burns lupines");

        // An empty tag set resolves to nothing.
        assert!(pools.resolve(&[]).is_empty());
        assert_eq!(pools.tag_count(), 2);
        assert!(pools.get("DOCKS").is_some());
        assert!(pools.remove("docks"));
        assert!(pools.get("docks").is_none());
    }

    #[test]
    fn pool_key_slugs_the_tag() {
        assert_eq!(pool_key("The Docks!").as_str(), "lore:the_docks");
        assert_eq!(pool_key("occult").as_str(), "lore:occult");
    }

    #[test]
    fn familiarity_thresholds_track_notes() {
        let mut store = LongTermStore::default();
        let npc = ObjId(9);
        // No record at all: Stranger.
        assert_eq!(
            familiarity_of(&store, npc, "Alice"),
            Familiarity::Stranger
        );
        // One note: Acquaintance.
        store.remember(npc, "Alice", "asked about the cellar", 100);
        assert_eq!(
            familiarity_of(&store, npc, "Alice"),
            Familiarity::Acquaintance
        );
        // Two notes: still Acquaintance.
        store.remember(npc, "Alice", "tips well", 200);
        assert_eq!(
            familiarity_of(&store, npc, "Alice"),
            Familiarity::Acquaintance
        );
        // Three notes: Known.
        store.remember(npc, "Alice", "owes me money", 300);
        assert_eq!(familiarity_of(&store, npc, "Alice"), Familiarity::Known);
        // Matching is slug-based, like recall.
        assert_eq!(familiarity_of(&store, npc, "alice"), Familiarity::Known);
        // A different NPC shares nothing.
        assert_eq!(
            familiarity_of(&store, ObjId(10), "Alice"),
            Familiarity::Stranger
        );
    }
}
