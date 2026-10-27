//! Personal relationships are engine observations and adjudicated consequences,
//! not a count of model-written memories. Receipts are never evicted: a full
//! ledger refuses new effects rather than making an old reward replayable.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::Familiarity;

/// One distinct encounter at most per real-world hour.
pub const CONTACT_BUCKET_SECONDS: i64 = 3600;
/// Maximum people whose relationships one NPC retains.
pub const MAX_RELATIONSHIPS_PER_NPC: usize = 256;
/// Permanent receipts per relationship; no eviction or reset path.
pub const MAX_RELATIONSHIP_EFFECTS: usize = 128;
/// Maximum encoded bytes in one complete relationship document.
pub const MAX_RELATIONSHIP_BYTES: usize = 128 * 1024;
/// The minimum personal trust for the private guarded-lore service.
pub const GUARDED_LORE_TRUST: i16 = 20;

/// Who authorized a durable change. Models have no variant or writing tool.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipSource {
    /// A game owner's confirmed completed-work receipt.
    CompletedWork,
    /// Explicit staff adjudication with an auditable author and reason.
    Staff { actor: ObjId },
}

/// Permanent receipt and explanation for an applied change.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelationshipEffect {
    pub cause: String,
    pub reason: String,
    pub source: RelationshipSource,
    pub trust_delta: i16,
    pub disposition_delta: i16,
    pub at: i64,
}

/// One identity-keyed relationship. Display names may change without losing it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelationshipRecord {
    pub schema: u16,
    pub npc: ObjId,
    pub character: ObjId,
    pub encounters: u16,
    pub first_met: Option<i64>,
    pub last_met: Option<i64>,
    pub last_contact_bucket: Option<i64>,
    pub trust: i16,
    pub disposition: i16,
    pub effects: Vec<RelationshipEffect>,
}

/// Safe prompt projection; receipt reasons and staff audit metadata stay private.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonalRelation {
    pub trust: i16,
    pub disposition: i16,
}

/// Public institutional assessment, independent of personal trust and access.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicStanding {
    pub name: String,
    pub score: i16,
    pub band: String,
}

/// Persistence key, independent of character names.
#[must_use]
pub fn relationship_key(npc: ObjId, character: ObjId) -> SmolStr {
    SmolStr::new(format!("relation:{}:{}", npc.0, character.0))
}

impl RelationshipRecord {
    fn new(npc: ObjId, character: ObjId) -> Self {
        Self {
            schema: 1,
            npc,
            character,
            encounters: 0,
            first_met: None,
            last_met: None,
            last_contact_bucket: None,
            trust: 0,
            disposition: 0,
            effects: Vec::new(),
        }
    }

    /// Familiarity comes only from distinct perceived contact buckets.
    #[must_use]
    pub fn familiarity(&self) -> Familiarity {
        match self.encounters {
            0 => Familiarity::Stranger,
            1..=2 => Familiarity::Acquaintance,
            _ => Familiarity::Known,
        }
    }

    /// Check a complete document, including recomputing scores from permanent receipts.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != 1
            || self.npc == self.character
            || self.effects.len() > MAX_RELATIONSHIP_EFFECTS
        {
            return Err("Invalid relationship schema, identities, or receipt count".into());
        }
        match (
            self.encounters,
            self.first_met,
            self.last_met,
            self.last_contact_bucket,
        ) {
            (0, None, None, None) => {}
            (count, Some(first), Some(last), Some(bucket))
                if count > 0
                    && first >= 0
                    && last >= first
                    && bucket == last / CONTACT_BUCKET_SECONDS
                    && u64::from(count) <= (bucket - first / CONTACT_BUCKET_SECONDS + 1) as u64 => {
            }
            _ => return Err("Invalid relationship encounter history".into()),
        }
        let mut causes = BTreeSet::new();
        let (mut trust, mut disposition) = (0_i16, 0_i16);
        for effect in &self.effects {
            validate_effect(effect)?;
            if !causes.insert(&effect.cause) {
                return Err("Duplicate relationship receipt".into());
            }
            trust = adjusted(trust, effect.trust_delta);
            disposition = adjusted(disposition, effect.disposition_delta);
        }
        if self.trust != trust || self.disposition != disposition {
            return Err("Relationship scores disagree with receipts".into());
        }
        if serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_RELATIONSHIP_BYTES
        {
            return Err("Relationship document exceeds its byte limit".into());
        }
        Ok(())
    }
}

fn adjusted(value: i16, delta: i16) -> i16 {
    value.saturating_add(delta).clamp(-100, 100)
}

fn validate_effect(effect: &RelationshipEffect) -> Result<(), String> {
    if effect.cause.is_empty()
        || effect.cause.len() > 128
        || !effect
            .cause
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_:./".contains(&byte))
        || effect.reason.trim().is_empty()
        || effect.reason.len() > 512
        || effect.at < 0
        || !(-100..=100).contains(&effect.trust_delta)
        || !(-100..=100).contains(&effect.disposition_delta)
        || (matches!(effect.source, RelationshipSource::CompletedWork)
            && (effect.trust_delta != 10 || effect.disposition_delta != 5))
    {
        return Err("Invalid relationship cause, reason, or effect".into());
    }
    Ok(())
}

/// Owner-held, bounded personal relationship records.
#[derive(Clone, Debug, Default)]
pub struct RelationshipStore {
    records: BTreeMap<(ObjId, ObjId), RelationshipRecord>,
}

impl RelationshipStore {
    #[must_use]
    pub fn record_for(&self, npc: ObjId, character: ObjId) -> Option<&RelationshipRecord> {
        self.records.get(&(npc, character))
    }

    #[must_use]
    pub fn personal(&self, npc: ObjId, character: ObjId) -> PersonalRelation {
        self.record_for(npc, character)
            .map_or_else(PersonalRelation::default, |record| PersonalRelation {
                trust: record.trust,
                disposition: record.disposition,
            })
    }

    #[must_use]
    pub fn familiarity(&self, npc: ObjId, character: ObjId) -> Familiarity {
        self.record_for(npc, character)
            .map_or(Familiarity::Stranger, RelationshipRecord::familiarity)
    }

    fn candidate(&self, npc: ObjId, character: ObjId) -> Result<RelationshipRecord, String> {
        if npc == character {
            return Err("An NPC cannot have a personal relationship with itself".into());
        }
        if let Some(record) = self.record_for(npc, character) {
            return Ok(record.clone());
        }
        if self
            .records
            .keys()
            .filter(|(owner, _)| *owner == npc)
            .count()
            >= MAX_RELATIONSHIPS_PER_NPC
        {
            return Err("NPC relationship capacity reached".into());
        }
        Ok(RelationshipRecord::new(npc, character))
    }

    pub fn insert(&mut self, record: RelationshipRecord) -> Result<(), String> {
        record.check()?;
        self.candidate(record.npc, record.character)?;
        if self
            .record_for(record.npc, record.character)
            .is_some_and(|old| old != &record)
        {
            return Err("Conflicting duplicate relationship document".into());
        }
        self.records.insert((record.npc, record.character), record);
        Ok(())
    }

    /// A repeated contact in the same/earlier time bucket changes nothing.
    pub fn contact(
        &mut self,
        npc: ObjId,
        character: ObjId,
        now: i64,
    ) -> Result<Option<RelationshipRecord>, String> {
        if now < 0 {
            return Err("Invalid relationship time".into());
        }
        let mut record = self.candidate(npc, character)?;
        let bucket = now / CONTACT_BUCKET_SECONDS;
        if record.last_contact_bucket.is_some_and(|old| old >= bucket) {
            return Ok(None);
        }
        record.encounters = record.encounters.saturating_add(1);
        record.first_met.get_or_insert(now);
        record.last_met = Some(now);
        record.last_contact_bucket = Some(bucket);
        record.check()?;
        self.records.insert((npc, character), record.clone());
        Ok(Some(record))
    }

    /// Exact retries return None even at capacity. Conflict checks precede writes.
    pub fn apply(
        &mut self,
        npc: ObjId,
        character: ObjId,
        effect: RelationshipEffect,
    ) -> Result<Option<RelationshipRecord>, String> {
        validate_effect(&effect)?;
        let mut record = self.candidate(npc, character)?;
        if let Some(old) = record.effects.iter().find(|old| old.cause == effect.cause) {
            if old.reason == effect.reason
                && old.source == effect.source
                && old.trust_delta == effect.trust_delta
                && old.disposition_delta == effect.disposition_delta
            {
                return Ok(None);
            }
            return Err("Relationship cause already records a different effect".into());
        }
        if record.effects.len() >= MAX_RELATIONSHIP_EFFECTS {
            return Err("Permanent relationship receipt ledger is full".into());
        }
        record.trust = adjusted(record.trust, effect.trust_delta);
        record.disposition = adjusted(record.disposition, effect.disposition_delta);
        record.effects.push(effect);
        record.check()?;
        self.records.insert((npc, character), record.clone());
        Ok(Some(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn work(cause: &str) -> RelationshipEffect {
        RelationshipEffect {
            cause: cause.into(),
            reason: "Completed verified work".into(),
            source: RelationshipSource::CompletedWork,
            trust_delta: 10,
            disposition_delta: 5,
            at: 10,
        }
    }
    #[test]
    fn repeated_speech_only_makes_an_acquaintance() {
        let mut store = RelationshipStore::default();
        for _ in 0..100 {
            store.contact(ObjId(1), ObjId(2), 10).expect("contact");
        }
        assert_eq!(
            store.familiarity(ObjId(1), ObjId(2)),
            Familiarity::Acquaintance
        );
        assert_eq!(
            store.personal(ObjId(1), ObjId(2)),
            PersonalRelation::default()
        );
        for hour in [1, 2] {
            store
                .contact(ObjId(1), ObjId(2), hour * CONTACT_BUCKET_SECONDS)
                .expect("later contact");
        }
        assert_eq!(store.familiarity(ObjId(1), ObjId(2)), Familiarity::Known);
        assert_eq!(store.personal(ObjId(1), ObjId(2)).trust, 0);
    }
    #[test]
    fn permanent_receipts_replay_conflict_and_reload() {
        let mut store = RelationshipStore::default();
        let record = store
            .apply(ObjId(1), ObjId(2), work("survey:1"))
            .expect("apply")
            .expect("fresh");
        let mut reload = RelationshipStore::default();
        reload
            .insert(
                serde_json::from_slice(&serde_json::to_vec(&record).expect("encode"))
                    .expect("decode"),
            )
            .expect("load");
        assert_eq!(
            reload
                .apply(ObjId(1), ObjId(2), work("survey:1"))
                .expect("replay"),
            None
        );
        let mut conflicting = work("survey:1");
        conflicting.reason = "Another event".into();
        assert!(reload.apply(ObjId(1), ObjId(2), conflicting).is_err());
        assert_eq!(reload.record_for(ObjId(1), ObjId(2)), Some(&record));
        assert_eq!(
            reload.familiarity(ObjId(1), ObjId(2)),
            Familiarity::Stranger
        );
    }
    #[test]
    fn trust_and_disposition_are_independent_and_full_ledger_never_replays() {
        let mut store = RelationshipStore::default();
        let effect = RelationshipEffect {
            cause: "staff:1".into(),
            reason: "Reliable but personally disliked".into(),
            source: RelationshipSource::Staff { actor: ObjId(3) },
            trust_delta: 30,
            disposition_delta: -40,
            at: 10,
        };
        store
            .apply(ObjId(1), ObjId(2), effect)
            .expect("staff effect");
        assert_eq!(
            store.personal(ObjId(1), ObjId(2)),
            PersonalRelation {
                trust: 30,
                disposition: -40
            }
        );
        for n in 1..MAX_RELATIONSHIP_EFFECTS {
            store
                .apply(ObjId(1), ObjId(2), work(&format!("work:{n}")))
                .expect("receipt");
        }
        assert!(store.apply(ObjId(1), ObjId(2), work("over-cap")).is_err());
        assert_eq!(
            store
                .apply(ObjId(1), ObjId(2), work("work:1"))
                .expect("replay at cap"),
            None
        );
        assert!(
            store
                .record_for(ObjId(1), ObjId(2))
                .expect("record")
                .check()
                .is_ok()
        );
    }
    #[test]
    fn forged_scores_and_duplicate_causes_fail_boot() {
        let mut store = RelationshipStore::default();
        let record = store
            .apply(ObjId(1), ObjId(2), work("one"))
            .expect("apply")
            .expect("record");
        let mut forged = record.clone();
        forged.trust = 100;
        assert!(RelationshipStore::default().insert(forged).is_err());
        let mut duplicate = record;
        duplicate.effects.push(work("one"));
        assert!(RelationshipStore::default().insert(duplicate).is_err());
    }
}
