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

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::memory::{self, LongTermStore};

/// The attribute an NPC's [`NpcKnowledge`] is stored under (JSON), in the same
/// `agent` namespace as [`PROFILE_ATTR`](crate::profile::PROFILE_ATTR).
pub const KNOWLEDGE_ATTR: &str = "agent:knowledge";

/// Maximum facts authored directly on one NPC.
pub const MAX_NPC_LORE_ENTRIES: usize = 64;
/// Maximum shared-pool tags attached to one NPC.
pub const MAX_NPC_LORE_TAGS: usize = 16;
/// Maximum shared lore pools loaded by one server.
pub const MAX_LORE_POOLS: usize = 512;
/// Maximum facts in one shared lore pool.
pub const MAX_LORE_POOL_ENTRIES: usize = 128;
/// Maximum subject bytes in one lore fact.
pub const MAX_LORE_SUBJECT_BYTES: usize = 256;
/// Maximum fact bytes in one lore fact.
pub const MAX_LORE_FACT_BYTES: usize = 2 * 1024;
/// Maximum provenance bytes in one lore fact.
pub const MAX_LORE_SOURCE_BYTES: usize = 512;
/// Maximum bytes in one pool tag before canonicalization.
pub const MAX_LORE_TAG_BYTES: usize = 128;
/// Maximum encoded bytes in one NPC knowledge document or shared pool.
pub const MAX_LORE_DOCUMENT_BYTES: usize = 256 * 1024;
/// Maximum shareable facts copied into one turn snapshot.
pub const MAX_RESOLVED_LORE_ENTRIES: usize = 128;
/// Maximum encoded fact bytes copied into one turn snapshot.
pub const MAX_RESOLVED_LORE_BYTES: usize = 256 * 1024;

/// Invalid lore or knowledge persisted/input shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoreValidationError {
    InvalidTag,
    TagTooLarge,
    DuplicateTag,
    TooManyPools,
    TooManyPoolEntries,
    TooManyNpcEntries,
    TooManyNpcTags,
    SubjectTooLarge,
    InvalidFact,
    FactTooLarge,
    SourceTooLarge,
    DocumentTooLarge,
}

impl fmt::Display for LoreValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTag => "lore tag must produce a non-empty canonical key",
            Self::TagTooLarge => "lore tag exceeds its byte limit",
            Self::DuplicateTag => "NPC knowledge contains a duplicate canonical pool tag",
            Self::TooManyPools => "shared lore pool count exceeds its hard limit",
            Self::TooManyPoolEntries => "shared lore pool entry count exceeds its hard limit",
            Self::TooManyNpcEntries => "NPC lore entry count exceeds its hard limit",
            Self::TooManyNpcTags => "NPC lore tag count exceeds its hard limit",
            Self::SubjectTooLarge => "lore subject exceeds its byte limit",
            Self::InvalidFact => "lore fact must not be empty",
            Self::FactTooLarge => "lore fact exceeds its byte limit",
            Self::SourceTooLarge => "lore provenance exceeds its byte limit",
            Self::DocumentTooLarge => "encoded lore document exceeds its byte limit",
        })
    }
}

impl std::error::Error for LoreValidationError {}

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

    /// Validate the complete fact before it is stored or projected.
    pub fn validate(&self) -> Result<(), LoreValidationError> {
        if self.subject.len() > MAX_LORE_SUBJECT_BYTES {
            return Err(LoreValidationError::SubjectTooLarge);
        }
        if self.fact.trim().is_empty() {
            return Err(LoreValidationError::InvalidFact);
        }
        if self.fact.len() > MAX_LORE_FACT_BYTES {
            return Err(LoreValidationError::FactTooLarge);
        }
        if self.source.len() > MAX_LORE_SOURCE_BYTES {
            return Err(LoreValidationError::SourceTooLarge);
        }
        Ok(())
    }

    fn encoded_len(&self) -> Result<usize, LoreValidationError> {
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|_| LoreValidationError::DocumentTooLarge)
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

impl NpcKnowledge {
    /// Validate count, field, duplicate-tag, and complete-document bounds.
    pub fn validate(&self) -> Result<(), LoreValidationError> {
        if self.lore.len() > MAX_NPC_LORE_ENTRIES {
            return Err(LoreValidationError::TooManyNpcEntries);
        }
        if self.tags.len() > MAX_NPC_LORE_TAGS {
            return Err(LoreValidationError::TooManyNpcTags);
        }
        for entry in &self.lore {
            entry.validate()?;
        }
        let mut tags = BTreeSet::new();
        for tag in &self.tags {
            let canonical = canonical_tag(tag)?;
            if !tags.insert(canonical) {
                return Err(LoreValidationError::DuplicateTag);
            }
        }
        validate_document_size(self)
    }
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

impl LorePool {
    /// Validate tag, entry, and complete-document bounds.
    pub fn validate(&self) -> Result<(), LoreValidationError> {
        canonical_tag(&self.tag)?;
        if self.entries.len() > MAX_LORE_POOL_ENTRIES {
            return Err(LoreValidationError::TooManyPoolEntries);
        }
        for entry in &self.entries {
            entry.validate()?;
        }
        validate_document_size(self)
    }
}

/// The persistence key one [`LorePool`] lives under (tag slugged so it is a safe
/// key), mirroring [`record_key`](crate::memory::record_key)'s shape.
#[must_use]
pub fn pool_key(tag: &str) -> SmolStr {
    SmolStr::new(format!("lore:{}", memory::slug(tag)))
}

/// Validate a staff/persisted pool tag without exposing its canonical key.
pub fn validate_lore_tag(tag: &str) -> Result<(), LoreValidationError> {
    canonical_tag(tag).map(|_| ())
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
    pub fn insert(&mut self, pool: LorePool) -> Result<(), LoreValidationError> {
        pool.validate()?;
        let key = canonical_tag(&pool.tag)?;
        if !self.pools.contains_key(&key) && self.pools.len() >= MAX_LORE_POOLS {
            return Err(LoreValidationError::TooManyPools);
        }
        self.pools.insert(key, pool);
        Ok(())
    }

    /// Insert or replace the pool for its tag (staff edit), keeping the in-RAM
    /// view in step with a persisted write.
    pub fn set(&mut self, pool: LorePool) -> Result<(), LoreValidationError> {
        self.insert(pool)
    }

    /// The pool for `tag`, if any.
    #[must_use]
    pub fn get(&self, tag: &str) -> Option<&LorePool> {
        canonical_tag(tag)
            .ok()
            .and_then(|canonical| self.pools.get(&canonical))
    }

    /// Remove the pool for `tag`. Returns whether one was present.
    pub fn remove(&mut self, tag: &str) -> bool {
        canonical_tag(tag)
            .ok()
            .is_some_and(|canonical| self.pools.remove(&canonical).is_some())
    }

    /// The concatenated entries of every pool matching `tags`, in a
    /// deterministic order (the given tag order, then each pool's entry order).
    /// Unknown tags are ignored.
    #[must_use]
    pub fn resolve(&self, tags: &[String]) -> Vec<LoreEntry> {
        self.resolve_matching(tags, |_| true)
    }

    /// Resolve only entries permitted to reach an NPC prompt. Filtering occurs
    /// before count/byte admission so a secret-heavy pool cannot starve later
    /// public facts from the bounded projection.
    #[must_use]
    pub fn resolve_shareable(&self, tags: &[String]) -> Vec<LoreEntry> {
        self.resolve_matching(tags, LoreEntry::is_shareable)
    }

    fn resolve_matching(
        &self,
        tags: &[String],
        include: impl Fn(&LoreEntry) -> bool,
    ) -> Vec<LoreEntry> {
        let mut out = Vec::new();
        let mut bytes = 0_usize;
        for tag in tags.iter().take(MAX_NPC_LORE_TAGS) {
            let Ok(tag) = canonical_tag(tag) else {
                continue;
            };
            if let Some(pool) = self.pools.get(&tag) {
                for entry in &pool.entries {
                    if !include(entry) {
                        continue;
                    }
                    if out.len() >= MAX_RESOLVED_LORE_ENTRIES {
                        return out;
                    }
                    let Ok(entry_bytes) = entry.encoded_len() else {
                        continue;
                    };
                    let Some(next_bytes) = bytes.checked_add(entry_bytes) else {
                        return out;
                    };
                    if next_bytes > MAX_RESOLVED_LORE_BYTES {
                        return out;
                    }
                    bytes = next_bytes;
                    out.push(entry.clone());
                }
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

fn canonical_tag(tag: &str) -> Result<String, LoreValidationError> {
    if tag.len() > MAX_LORE_TAG_BYTES {
        return Err(LoreValidationError::TagTooLarge);
    }
    let canonical = memory::slug(tag);
    if canonical.is_empty() {
        return Err(LoreValidationError::InvalidTag);
    }
    Ok(canonical)
}

fn validate_document_size(value: &impl Serialize) -> Result<(), LoreValidationError> {
    let encoded = serde_json::to_vec(value).map_err(|_| LoreValidationError::DocumentTooLarge)?;
    if encoded.len() > MAX_LORE_DOCUMENT_BYTES {
        return Err(LoreValidationError::DocumentTooLarge);
    }
    Ok(())
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
        pools
            .insert(LorePool {
                tag: "Docks".into(),
                entries: vec![entry("smugglers land at midnight")],
            })
            .unwrap();
        pools
            .insert(LorePool {
                tag: "occult".into(),
                entries: vec![
                    entry("vampires shun sunlight"),
                    entry("silver burns lupines"),
                ],
            })
            .unwrap();

        // Tag matching is slug-insensitive; unknown tags contribute nothing.
        let resolved = pools.resolve(&["docks".into(), "occult".into(), "no-such-pool".into()]);
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
        assert_eq!(familiarity_of(&store, npc, "Alice"), Familiarity::Stranger);
        // One note: Acquaintance.
        store
            .remember(npc, "Alice", "asked about the cellar", 100)
            .unwrap();
        assert_eq!(
            familiarity_of(&store, npc, "Alice"),
            Familiarity::Acquaintance
        );
        // Two notes: still Acquaintance.
        store.remember(npc, "Alice", "tips well", 200).unwrap();
        assert_eq!(
            familiarity_of(&store, npc, "Alice"),
            Familiarity::Acquaintance
        );
        // Three notes: Known.
        store.remember(npc, "Alice", "owes me money", 300).unwrap();
        assert_eq!(familiarity_of(&store, npc, "Alice"), Familiarity::Known);
        // Matching is slug-based, like recall.
        assert_eq!(familiarity_of(&store, npc, "alice"), Familiarity::Known);
        // A different NPC shares nothing.
        assert_eq!(
            familiarity_of(&store, ObjId(10), "Alice"),
            Familiarity::Stranger
        );
    }

    #[test]
    fn knowledge_rejects_duplicate_tags_and_count_overflow() {
        let duplicate = NpcKnowledge {
            tags: vec!["The Docks".into(), "the_docks".into()],
            ..NpcKnowledge::default()
        };
        assert_eq!(duplicate.validate(), Err(LoreValidationError::DuplicateTag));

        let too_many = NpcKnowledge {
            lore: (0..=MAX_NPC_LORE_ENTRIES)
                .map(|index| entry(&format!("fact {index}")))
                .collect(),
            ..NpcKnowledge::default()
        };
        assert_eq!(
            too_many.validate(),
            Err(LoreValidationError::TooManyNpcEntries)
        );
    }

    #[test]
    fn pool_and_projection_have_independent_hard_caps() {
        let mut pools = LorePools::default();
        pools
            .insert(LorePool {
                tag: "first".into(),
                entries: (0..MAX_LORE_POOL_ENTRIES)
                    .map(|index| entry(&format!("first {index}")))
                    .collect(),
            })
            .unwrap();
        pools
            .insert(LorePool {
                tag: "second".into(),
                entries: (0..MAX_LORE_POOL_ENTRIES)
                    .map(|index| entry(&format!("second {index}")))
                    .collect(),
            })
            .unwrap();
        let resolved = pools.resolve(&["first".into(), "second".into()]);
        assert_eq!(resolved.len(), MAX_RESOLVED_LORE_ENTRIES);
        assert!(resolved.iter().all(|entry| entry.fact.starts_with("first")));

        let oversized = LorePool {
            tag: "oversized".into(),
            entries: (0..=MAX_LORE_POOL_ENTRIES)
                .map(|index| entry(&format!("fact {index}")))
                .collect(),
        };
        assert_eq!(
            oversized.validate(),
            Err(LoreValidationError::TooManyPoolEntries)
        );
    }

    #[test]
    fn secret_entries_do_not_consume_shareable_projection_capacity() {
        let mut pools = LorePools::default();
        pools
            .insert(LorePool {
                tag: "secret".into(),
                entries: (0..MAX_LORE_POOL_ENTRIES)
                    .map(|index| LoreEntry {
                        fact: format!("secret {index}"),
                        secrecy: Secrecy::Secret,
                        ..LoreEntry::default()
                    })
                    .collect(),
            })
            .unwrap();
        pools
            .insert(LorePool {
                tag: "public".into(),
                entries: vec![entry("visible fact")],
            })
            .unwrap();
        let resolved = pools.resolve_shareable(&["secret".into(), "public".into()]);
        assert_eq!(resolved, vec![entry("visible fact")]);
    }
}
