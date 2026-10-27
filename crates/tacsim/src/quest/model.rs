//! Frozen authored quest terms and owner-verified, ordered progress receipts.
use crate::port::passenger::store::{Booking, Phase};
use mush_core::{CanonicalHash, CanonicalId, ObjId, World};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA: u16 = 1;
pub const MAX_CHECKPOINTS: usize = 8;
pub const MAX_DETAIL_BYTES: usize = 2048;
pub const MAX_DEFINITION_BYTES: usize = 12 * 1024;
pub const MAX_RECORD_BYTES: usize = 24 * 1024;
pub const MAX_INDEX_BYTES: usize = 32 * 1024;
pub const MAX_QUESTS: usize = 128;

fn text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.contains('\0')
}
fn canonical(value: &str, maximum: usize) -> bool {
    text(value, maximum) && CanonicalId::new(value).is_ok()
}
fn source(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
}
fn within<T: Serialize>(value: &T, maximum: usize) -> bool {
    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= maximum)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestDef {
    pub schema: u16,
    pub key: String,
    pub title: String,
    pub introduction: String,
    pub destination: String,
    pub checkpoints: Vec<Checkpoint>,
    pub completion: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub id: String,
    pub title: String,
    pub room_pack: String,
    pub room_entry: String,
    pub room: ObjId,
    pub detail: String,
    pub instructions: String,
}
impl Checkpoint {
    pub fn check(&self) -> Result<(), String> {
        if !canonical(&self.id, 64)
            || !text(&self.title, 128)
            || !canonical(&self.room_pack, 160)
            || !canonical(&self.room_entry, 160)
            || self.room.0 == 0
            || !text(&self.detail, 128)
            || !text(&self.instructions, 1024)
        {
            Err("invalid quest checkpoint".into())
        } else {
            Ok(())
        }
    }
}
impl QuestDef {
    pub fn check(&self) -> Result<(), String> {
        if self.schema != SCHEMA
            || !source(&self.key)
            || !text(&self.title, 128)
            || !text(&self.introduction, 2048)
            || !canonical(&self.destination, 64)
            || self.checkpoints.is_empty()
            || self.checkpoints.len() > MAX_CHECKPOINTS
            || !text(&self.completion, 512)
        {
            return Err("invalid quest definition".into());
        }
        let mut ids = BTreeSet::new();
        for checkpoint in &self.checkpoints {
            checkpoint.check()?;
            if !ids.insert(&checkpoint.id) {
                return Err("quest checkpoint IDs must be unique".into());
            }
        }
        if !within(self, MAX_DEFINITION_BYTES) {
            return Err("quest definition exceeds its byte bound".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestKey {
    pub source: String,
    pub booking: u64,
}
impl QuestKey {
    pub fn check(&self) -> Result<(), String> {
        if source(&self.source) && self.booking <= i64::MAX as u64 {
            Ok(())
        } else {
            Err("invalid quest key".into())
        }
    }
    pub fn key_text(&self) -> String {
        format!("{}/{}", self.source, self.booking)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub at_utc: i64,
    pub at_ic: i64,
}
impl Stamp {
    pub fn now(w: &World) -> Self {
        Self {
            at_utc: w.now(),
            at_ic: w.world_ic_now().at,
        }
    }
    fn follows(&self, before: &Self) -> bool {
        self.at_utc >= before.at_utc && self.at_ic >= before.at_ic
    }
}
/// Hash exact server-read public detail text, retaining it alongside the digest.
pub fn detail_hash(value: &str) -> Result<String, String> {
    if !text(value, MAX_DETAIL_BYTES) {
        return Err("quest detail text is empty or too large".into());
    }
    CanonicalHash::for_canonical_value(
        "tacsim.quest_detail.v1",
        1,
        &serde_json::Value::String(value.to_owned()),
        16 * 1024,
    )
    .map(|hash| hash.to_hex())
    .map_err(|_| "quest detail cannot be hashed".into())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckEvidence {
    pub checkpoint: String,
    pub room: ObjId,
    pub detail_hash: String,
    pub detail_text: String,
    pub at: Stamp,
}
impl CheckEvidence {
    pub fn check(&self) -> Result<(), String> {
        if !canonical(&self.checkpoint, 64)
            || self.room.0 == 0
            || self.detail_hash != detail_hash(&self.detail_text)?
        {
            return Err("invalid quest check evidence".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestRecord {
    pub schema: u16,
    pub key: QuestKey,
    pub revision: u64,
    pub actor: ObjId,
    pub npc: ObjId,
    pub passage: Booking,
    pub definition: QuestDef,
    pub accepted: Stamp,
    pub checks: Vec<CheckEvidence>,
    pub completed: Option<Stamp>,
}
impl QuestRecord {
    pub fn check(&self) -> Result<(), String> {
        self.key.check()?;
        self.definition.check()?;
        self.passage.check()?;
        let delivered = self
            .passage
            .delivered
            .as_ref()
            .ok_or("quest passage has not arrived")?;
        if self.schema != SCHEMA
            || self.actor.0 == 0
            || self.npc.0 == 0
            || self.actor == self.npc
            || self.actor != self.passage.captain
            || self.npc != self.passage.npc
            || self.passage.phase != Phase::Delivered
            || self.key.booking != self.passage.id
            || self.key.source != self.definition.key
            || self.definition.destination != self.passage.destination
            || self.accepted.at_ic < delivered.at_ic
            || self.accepted.at_utc < delivered.at_utc
            || self.checks.len() > self.definition.checkpoints.len()
            || self.revision != 1 + self.checks.len() as u64 + u64::from(self.completed.is_some())
            || (self.completed.is_some() && self.checks.len() != self.definition.checkpoints.len())
        {
            return Err("invalid quest progress receipt".into());
        }
        let mut previous = &self.accepted;
        for (evidence, checkpoint) in self.checks.iter().zip(&self.definition.checkpoints) {
            evidence.check()?;
            if evidence.checkpoint != checkpoint.id
                || evidence.room != checkpoint.room
                || !evidence.at.follows(previous)
            {
                return Err("quest checks are out of order".into());
            }
            previous = &evidence.at;
        }
        if self
            .completed
            .as_ref()
            .is_some_and(|done| !done.follows(previous))
        {
            return Err("quest completion predates its checks".into());
        }
        if !within(self, MAX_RECORD_BYTES) {
            return Err("quest receipt exceeds its byte bound".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestIndex {
    pub schema: u16,
    pub keys: Vec<QuestKey>,
}
impl Default for QuestIndex {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            keys: Vec::new(),
        }
    }
}
impl QuestIndex {
    pub fn check(&self) -> Result<(), String> {
        if self.schema != SCHEMA
            || self.keys.len() > MAX_QUESTS
            || self.keys.windows(2).any(|pair| pair.first() >= pair.get(1))
        {
            return Err("invalid quest index".into());
        }
        for key in &self.keys {
            key.check()?;
        }
        if !within(self, MAX_INDEX_BYTES) {
            return Err("quest index exceeds its byte bound".into());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;
    pub(crate) fn record() -> QuestRecord {
        let npc = ObjId(10);
        let actor = ObjId(11);
        QuestRecord {
            schema: 1,
            key: QuestKey {
                source: "pack/quest.leila".into(),
                booking: 0,
            },
            revision: 1,
            actor,
            npc,
            passage: Booking {
                schema: 1,
                id: 0,
                revision: 3,
                npc,
                captain: actor,
                hull: ObjId(12),
                traveller_key: "pack/passenger.leila".into(),
                origin: "stickney".into(),
                destination: "ceres".into(),
                purse: 0,
                payee: 1,
                fare_minor: 30000,
                accepted_ic: 10,
                deadline_ic: 100,
                contract: 0,
                phase: Phase::Delivered,
                journey: Some(crate::port::passenger::store::JourneyProof {
                    route: "stickney_ceres".into(),
                    departed_ic: 20,
                    arrival_ic: 50,
                }),
                delivered: Some(crate::port::passenger::store::OutcomeStamp {
                    at_ic: 50,
                    at_utc: 50,
                }),
                cancelled: None,
            },
            definition: QuestDef {
                schema: 1,
                key: "pack/quest.leila".into(),
                title: "Calibration".into(),
                introduction: "Check the bench.".into(),
                destination: "ceres".into(),
                checkpoints: vec![Checkpoint {
                    id: "bench".into(),
                    title: "Check bench".into(),
                    room_pack: "neighborhood.ceres".into(),
                    room_entry: "room.workshop".into(),
                    room: ObjId(20),
                    detail: "bench".into(),
                    instructions: "Read the public scale.".into(),
                }],
                completion: "Completed the calibration checks together.".into(),
            },
            accepted: Stamp {
                at_ic: 60,
                at_utc: 60,
            },
            checks: Vec::new(),
            completed: None,
        }
    }
    #[test]
    fn ordered_evidence_binds_text_room_passage_and_completion() -> TestResult {
        let mut row = record();
        row.check()?;
        row.checks.push(CheckEvidence {
            checkpoint: "bench".into(),
            room: ObjId(20),
            detail_hash: detail_hash("Scale reads 42.")?,
            detail_text: "Scale reads 42.".into(),
            at: Stamp {
                at_ic: 70,
                at_utc: 70,
            },
        });
        row.revision = 2;
        row.check()?;
        row.completed = Some(Stamp {
            at_ic: 80,
            at_utc: 80,
        });
        row.revision = 3;
        row.check()?;
        let mut bad = row.clone();
        bad.checks.first_mut().ok_or("evidence")?.detail_text = "Invented result".into();
        check!(bad.check().is_err());
        bad = row.clone();
        bad.actor = ObjId(30);
        check!(bad.check().is_err());
        bad = row.clone();
        bad.completed = Some(Stamp {
            at_ic: 69,
            at_utc: 80,
        });
        check!(bad.check().is_err());
        bad = row;
        bad.checks.clear();
        bad.revision = 2;
        check!(bad.check().is_err());
        Ok(())
    }
    #[test]
    fn bounded_unique_definitions_and_permanent_index_reject_corruption() -> TestResult {
        let row = record();
        let mut def = row.definition.clone();
        def.checkpoints
            .push(def.checkpoints.first().ok_or("checkpoint")?.clone());
        check!(def.check().is_err());
        def = row.definition;
        def.completion = "é".repeat(257);
        check!(def.check().is_err());
        let mut index = QuestIndex {
            schema: 1,
            keys: vec![row.key.clone()],
        };
        index.check()?;
        index.keys.push(row.key);
        check!(index.check().is_err());
        check!(detail_hash(&"é".repeat(MAX_DETAIL_BYTES)).is_err());
        check!(
            serde_json::from_value::<QuestIndex>(
                serde_json::json!({"schema":1,"keys":[],"reset":true})
            )
            .is_err()
        );
        Ok(())
    }
}
