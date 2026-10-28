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
pub const MAX_QUESTS_PER_NPC: usize = 8;
pub const MAX_QUEST_CATALOG_BYTES: usize = 64 * 1024;

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
    #[serde(default, skip_serializing_if = "Prerequisite::is_paid_passage")]
    pub prerequisite: Prerequisite,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<QuestChoice>,
    pub checkpoints: Vec<Checkpoint>,
    pub completion: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Prerequisite {
    #[default]
    PaidPassage,
    Open,
    Completed {
        quest: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestChoice {
    pub group: String,
    pub path: String,
}
impl Prerequisite {
    fn is_paid_passage(&self) -> bool {
        *self == Self::PaidPassage
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Objective {
    #[default]
    Observation,
    CoopEnrolment,
    Trade {
        origin: String,
        destination: String,
        commodity: String,
        quantity: u64,
    },
    Passenger {
        npc: ObjId,
    },
    Contact {
        npc: ObjId,
    },
    PaidPassage {
        npc: ObjId,
        origin: String,
        destination: String,
    },
    Survey {
        mode: String,
    },
}
impl Objective {
    fn is_observation(&self) -> bool {
        *self == Self::Observation
    }
    pub fn check(&self) -> Result<(), String> {
        let valid = match self {
            Self::Observation | Self::CoopEnrolment => true,
            Self::Trade {
                origin,
                destination,
                commodity,
                quantity,
            } => {
                canonical(origin, 64)
                    && canonical(destination, 64)
                    && origin != destination
                    && canonical(commodity, 64)
                    && *quantity > 0
                    && *quantity <= i64::MAX as u64
            }
            Self::Passenger { npc } | Self::Contact { npc } => npc.0 != 0,
            Self::PaidPassage {
                npc,
                origin,
                destination,
            } => {
                npc.0 != 0
                    && canonical(origin, 64)
                    && canonical(destination, 64)
                    && origin != destination
            }
            Self::Survey { mode } => matches!(mode.as_str(), "scan" | "sound"),
        };
        valid
            .then_some(())
            .ok_or_else(|| "invalid quest objective".into())
    }
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
    #[serde(default, skip_serializing_if = "Objective::is_observation")]
    pub objective: Objective,
    pub instructions: String,
}
impl Checkpoint {
    pub fn check(&self) -> Result<(), String> {
        self.objective.check()?;
        if !canonical(&self.id, 64)
            || !text(&self.title, 128)
            || !canonical(&self.room_pack, 160)
            || !canonical(&self.room_entry, 160)
            || self.room.0 == 0
            || self.detail.len() > 128
            || self.detail.contains('\0')
            || (self.objective.is_observation() && !text(&self.detail, 128))
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
        if let Prerequisite::Completed { quest } = &self.prerequisite
            && (!source(quest) || quest == &self.key)
        {
            return Err("invalid completed quest prerequisite".into());
        }
        if let Some(choice) = &self.choice
            && (self.prerequisite == Prerequisite::PaidPassage
                || !source(&choice.group)
                || !canonical(&choice.path, 64))
        {
            return Err("invalid quest choice".into());
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

/// A direct legacy definition remains byte-compatible. Catalogs have a canonical
/// order so a staff edit cannot silently change which authored option is first.
pub(crate) fn decode_definitions(value: &serde_json::Value) -> Result<Vec<QuestDef>, String> {
    if !within(value, MAX_QUEST_CATALOG_BYTES) {
        return Err("quest catalog exceeds its byte bound".into());
    }
    let definitions: Vec<QuestDef> = if value.is_array() {
        serde_json::from_value(value.clone()).map_err(|_| "invalid quest catalog")?
    } else {
        vec![serde_json::from_value(value.clone()).map_err(|_| "invalid quest definition")?]
    };
    if definitions.is_empty()
        || definitions.len() > MAX_QUESTS_PER_NPC
        || definitions
            .windows(2)
            .any(|pair| pair.first().map(|def| &def.key) >= pair.get(1).map(|def| &def.key))
    {
        return Err("quest catalog must have bounded, unique, sorted keys".into());
    }
    for definition in &definitions {
        definition.check()?;
    }
    Ok(definitions)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestKey {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ObjId>,
}
impl QuestKey {
    pub fn check(&self) -> Result<(), String> {
        if source(&self.source)
            && matches!(
                (self.booking, self.actor),
                (Some(0..=0x7fff_ffff_ffff_ffff), None) | (None, Some(ObjId(1..)))
            )
        {
            Ok(())
        } else {
            Err("invalid quest key".into())
        }
    }
    pub fn key_text(&self) -> String {
        match (self.booking, self.actor) {
            (Some(booking), None) => format!("{}/{booking}", self.source),
            (None, Some(actor)) => format!("{}/actor/{}/open", self.source, actor.0),
            _ => String::new(),
        }
    }
}

/// The original global index keeps its canonical key and serialized key value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum QuestIndexKey {
    Legacy,
    Actor(ObjId),
}
impl QuestIndexKey {
    pub fn for_quest(key: &QuestKey) -> Self {
        key.actor.map(Self::Actor).unwrap_or(Self::Legacy)
    }
    pub fn key_text(&self) -> Result<String, String> {
        match self {
            Self::Legacy => Ok("0".into()),
            Self::Actor(actor) if actor.0 != 0 => Ok(format!("actor/{}", actor.0)),
            Self::Actor(_) => Err("invalid quest index actor".into()),
        }
    }
}
impl Serialize for QuestIndexKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Legacy => serializer.serialize_u8(0),
            Self::Actor(actor) => serializer.serialize_str(&format!("actor/{}", actor.0)),
        }
    }
}
impl<'de> Deserialize<'de> for QuestIndexKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.as_u64() == Some(0) {
            return Ok(Self::Legacy);
        }
        if let Some(raw) = value.as_str().and_then(|text| text.strip_prefix("actor/"))
            && let Ok(id) = raw.parse::<u32>()
            && id != 0
            && raw == id.to_string()
        {
            return Ok(Self::Actor(ObjId(id)));
        }
        Err(serde::de::Error::custom("invalid quest index key"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub at_utc: i64,
    pub at_ic: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosureKind {
    Declined,
    Abandoned,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedQuest {
    pub kind: ClosureKind,
    pub at: Stamp,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<super::evidence::ObjectiveProof>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passage: Option<Booking>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisite_receipt: Option<QuestKey>,
    pub definition: QuestDef,
    pub accepted: Stamp,
    pub checks: Vec<CheckEvidence>,
    pub completed: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<ClosedQuest>,
}
impl QuestRecord {
    pub fn is_active(&self) -> bool {
        self.completed.is_none() && self.closed.is_none()
    }

    pub fn check(&self) -> Result<(), String> {
        self.key.check()?;
        self.definition.check()?;
        match (&self.definition.prerequisite, &self.passage) {
            (Prerequisite::PaidPassage, Some(passage)) => {
                passage.check()?;
                let delivered = passage
                    .delivered
                    .as_ref()
                    .ok_or("quest passage has not arrived")?;
                if self.actor != passage.captain
                    || self.npc != passage.npc
                    || passage.phase != Phase::Delivered
                    || self.key.booking != Some(passage.id)
                    || self.key.actor.is_some()
                    || self.definition.destination != passage.destination
                    || self.accepted.at_ic < delivered.at_ic
                    || self.accepted.at_utc < delivered.at_utc
                {
                    return Err("quest passage does not match its receipt".into());
                }
            }
            (Prerequisite::Open | Prerequisite::Completed { .. }, None)
                if self.key.actor == Some(self.actor) && self.key.booking.is_none() => {}
            _ => return Err("quest prerequisite does not match its identity".into()),
        }
        match (&self.definition.prerequisite, &self.prerequisite_receipt) {
            (Prerequisite::Completed { quest }, Some(key)) => {
                key.check()?;
                if &key.source != quest
                    || key == &self.key
                    || key.actor.is_some_and(|actor| actor != self.actor)
                {
                    return Err(
                        "quest prerequisite receipt names another quest or character".into(),
                    );
                }
            }
            (Prerequisite::Completed { .. }, None) | (_, Some(_)) => {
                return Err("quest prerequisite receipt is missing or unexpected".into());
            }
            (_, None) => {}
        }
        let abandoned = self
            .closed
            .as_ref()
            .is_some_and(|closed| closed.kind == ClosureKind::Abandoned);
        if self.schema != SCHEMA
            || self.actor.0 == 0
            || self.npc.0 == 0
            || self.actor == self.npc
            || self.key.source != self.definition.key
            || self.checks.len() > self.definition.checkpoints.len()
            || self.revision
                != 1 + self.checks.len() as u64 + u64::from(self.completed.is_some() || abandoned)
            || (self.completed.is_some() && self.checks.len() != self.definition.checkpoints.len())
            || (self.completed.is_some() && self.closed.is_some())
        {
            return Err("invalid quest progress receipt".into());
        }
        let mut previous = &self.accepted;
        for (evidence, checkpoint) in self.checks.iter().zip(&self.definition.checkpoints) {
            evidence.check()?;
            match (&checkpoint.objective, &evidence.proof) {
                (Objective::Observation, None) => {}
                (Objective::Observation, Some(_)) | (_, None) => {
                    return Err("quest evidence has the wrong proof kind".into());
                }
                (objective, Some(proof)) => {
                    proof.check_for(objective, self.actor, &self.accepted, &evidence.at)?
                }
            }
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
        if let Some(closed) = &self.closed
            && (!closed.at.follows(previous)
                || (closed.kind == ClosureKind::Declined
                    && (self.revision != 1
                        || !self.checks.is_empty()
                        || closed.at != self.accepted)))
        {
            return Err("invalid quest closure receipt".into());
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
    pub fn check_for(&self, key: QuestIndexKey) -> Result<(), String> {
        self.check()?;
        key.key_text()?;
        if self
            .keys
            .iter()
            .any(|quest| QuestIndexKey::for_quest(quest) != key)
        {
            return Err("quest index contains another character's receipts".into());
        }
        Ok(())
    }
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
                booking: Some(0),
                actor: None,
            },
            revision: 1,
            actor,
            npc,
            passage: Some(Booking {
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
            }),
            prerequisite_receipt: None,
            definition: QuestDef {
                schema: 1,
                key: "pack/quest.leila".into(),
                title: "Calibration".into(),
                introduction: "Check the bench.".into(),
                destination: "ceres".into(),
                prerequisite: Prerequisite::PaidPassage,
                choice: None,
                checkpoints: vec![Checkpoint {
                    id: "bench".into(),
                    title: "Check bench".into(),
                    room_pack: "neighborhood.ceres".into(),
                    room_entry: "room.workshop".into(),
                    room: ObjId(20),
                    detail: "bench".into(),
                    objective: Objective::Observation,
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
            closed: None,
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
            proof: None,
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

    #[test]
    fn legacy_json_and_canonical_keys_keep_their_original_shape() -> TestResult {
        let row = record();
        let original = serde_json::to_value(&row)?;
        check_eq!(original["key"]["booking"], serde_json::json!(0));
        check!(original["key"].get("actor").is_none());
        check!(original["definition"].get("prerequisite").is_none());
        check!(original["definition"].get("choice").is_none());
        check!(original.get("prerequisite_receipt").is_none());
        check!(original.get("closed").is_none());
        check!(
            original["definition"]["checkpoints"][0]
                .get("objective")
                .is_none()
        );
        check!(original["passage"].is_object());
        check_eq!(row.key.key_text(), "pack/quest.leila/0");
        let decoded: QuestRecord = serde_json::from_value(original.clone())?;
        decoded.check()?;
        check_eq!(decoded, row);
        let mut explicit = original.clone();
        explicit["definition"]["prerequisite"] = serde_json::json!("paid_passage");
        explicit["definition"]["checkpoints"][0]["objective"] =
            serde_json::json!({"kind":"observation"});
        let decoded: QuestRecord = serde_json::from_value(explicit)?;
        check_eq!(serde_json::to_value(decoded)?, original);
        check_eq!(
            serde_json::to_value(QuestIndexKey::Legacy)?,
            serde_json::json!(0)
        );
        check_eq!(
            serde_json::from_value::<QuestIndexKey>(serde_json::json!(0))?,
            QuestIndexKey::Legacy
        );
        check_eq!(QuestIndexKey::Legacy.key_text()?, "0");
        check_eq!(QuestIndexKey::Actor(ObjId(11)).key_text()?, "actor/11");
        check!(serde_json::from_value::<QuestIndexKey>(serde_json::json!("actor/011")).is_err());
        Ok(())
    }

    #[test]
    fn open_quests_bind_one_character_without_a_passage_and_require_typed_objective_proof()
    -> TestResult {
        let mut row = record();
        row.definition.prerequisite = Prerequisite::Open;
        row.key.booking = None;
        row.key.actor = Some(row.actor);
        row.passage = None;
        row.check()?;
        check_eq!(row.key.key_text(), "pack/quest.leila/actor/11/open");
        let old_collision = QuestKey {
            source: "pack/quest.leila/actor".into(),
            booking: Some(11),
            actor: None,
        };
        check!(old_collision.key_text() != row.key.key_text());
        let mut bad = row.clone();
        bad.key.actor = Some(ObjId(12));
        check!(bad.check().is_err());
        bad = row.clone();
        bad.key.booking = Some(0);
        check!(bad.check().is_err());
        bad = row.clone();
        bad.key.actor = None;
        check!(bad.check().is_err());
        bad = row.clone();
        bad.passage = record().passage;
        check!(bad.check().is_err());
        let checkpoint = row.definition.checkpoints.first_mut().ok_or("checkpoint")?;
        checkpoint.objective = Objective::Survey {
            mode: "scan".into(),
        };
        checkpoint.detail.clear();
        row.check()?;
        row.checks.push(CheckEvidence {
            checkpoint: "bench".into(),
            room: ObjId(20),
            detail_hash: detail_hash("Completed a scan.")?,
            detail_text: "Completed a scan.".into(),
            at: Stamp {
                at_ic: 70,
                at_utc: 70,
            },
            proof: None,
        });
        row.revision = 2;
        check!(row.check().is_err());
        check!(
            Objective::Survey {
                mode: "invented".into()
            }
            .check()
            .is_err()
        );
        check!(
            Objective::Trade {
                origin: "ceres".into(),
                destination: "ceres".into(),
                commodity: "parts".into(),
                quantity: 1
            }
            .check()
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn catalogs_accept_legacy_and_bounded_sorted_unique_definitions() -> TestResult {
        let mut first = record().definition;
        let legacy = serde_json::to_value(&first)?;
        check_eq!(decode_definitions(&legacy)?, vec![first.clone()]);
        let mut second = first.clone();
        second.key = "pack/quest.next".into();
        second.prerequisite = Prerequisite::Completed {
            quest: first.key.clone(),
        };
        second.choice = Some(QuestChoice {
            group: "pack/leila_referral".into(),
            path: "trade".into(),
        });
        second.check()?;
        check_eq!(
            serde_json::to_value(&second.prerequisite)?,
            serde_json::json!({"completed":{"quest":first.key}})
        );
        let array = serde_json::to_value(vec![first.clone(), second.clone()])?;
        check_eq!(decode_definitions(&array)?.len(), 2);
        check!(
            decode_definitions(&serde_json::to_value(vec![second.clone(), first.clone()])?)
                .is_err()
        );
        check!(
            decode_definitions(&serde_json::to_value(vec![first.clone(), first.clone()])?).is_err()
        );
        check!(decode_definitions(&serde_json::json!([])).is_err());
        check!(
            decode_definitions(&serde_json::json!("x".repeat(MAX_QUEST_CATALOG_BYTES))).is_err()
        );
        first.choice = second.choice.clone();
        check!(first.check().is_err());
        second.prerequisite = Prerequisite::Completed {
            quest: second.key.clone(),
        };
        check!(second.check().is_err());
        check!(
            serde_json::from_value::<Prerequisite>(
                serde_json::json!({"completed":{"quest":"pack/quest.old","invented":true}})
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn closed_receipts_are_terminal_and_prerequisites_bind_exact_keys() -> TestResult {
        let before = record();
        let mut declined = before.clone();
        declined.closed = Some(ClosedQuest {
            kind: ClosureKind::Declined,
            at: declined.accepted.clone(),
        });
        declined.check()?;
        check!(!declined.is_active());
        let mut bad = declined.clone();
        bad.closed.as_mut().ok_or("closure")?.at.at_ic += 1;
        check!(bad.check().is_err());
        let mut abandoned = before.clone();
        abandoned.closed = Some(ClosedQuest {
            kind: ClosureKind::Abandoned,
            at: abandoned.accepted.clone(),
        });
        abandoned.revision = 2;
        abandoned.check()?;
        abandoned.completed = Some(abandoned.accepted.clone());
        check!(abandoned.check().is_err());

        let mut next = before.clone();
        next.key = QuestKey {
            source: "pack/quest.next".into(),
            booking: None,
            actor: Some(next.actor),
        };
        next.definition.key.clone_from(&next.key.source);
        next.definition.prerequisite = Prerequisite::Completed {
            quest: before.key.source.clone(),
        };
        next.passage = None;
        check!(next.check().is_err());
        next.prerequisite_receipt = Some(before.key.clone());
        next.check()?;
        next.prerequisite_receipt
            .as_mut()
            .ok_or("prerequisite")?
            .source = "pack/quest.other".into();
        check!(next.check().is_err());
        Ok(())
    }
}
