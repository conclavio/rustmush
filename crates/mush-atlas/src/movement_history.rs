//! The real Atlas knowledge mutation journal. Only Ledger open tails are live;
//! historical projection checks are transient boot work, never a World mirror.

use std::collections::BTreeSet;

use mush_core::participant::ParticipantError;
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::record::{MAX_KNOWN_PLACES, RecordKey};
use crate::travel::KnownDoc;

pub(crate) const LOG: &str = "movement_knowledge";
pub(crate) const MAX_BYTES: usize = 128 * 1024;
pub(crate) const RESIDENT_BYTES: usize = 1_048_576;

/// Atlas-owned open tails and composition configuration, with no retained
/// authoritative projection or cross-owner state.
#[derive(Clone, Debug, Default)]
pub struct MovementHistory {
    pub(crate) ledger: mush_core::Ledger,
    pub(crate) enabled: bool,
}

impl MovementHistory {
    pub(crate) fn fence(&self) -> Result<DefinitionDigest, ParticipantError> {
        if !self.enabled || !self.ledger.resident_within_bounds(1, RESIDENT_BYTES) {
            return Err(refused());
        }
        DefinitionDigest::for_canonical_value_with_limit(
            "atlas.movement.knowledge_tail.v1",
            &serde_json::json!({
                "target":self.ledger.chunk_target_bytes(), "next":self.ledger.next_seq(LOG),
                "chunk":self.ledger.open_chunk(LOG), "tail":self.ledger.open_entries(LOG)
            }),
            RESIDENT_BYTES,
        )
        .map_err(|_| ParticipantError::ImageLimit)
    }
    pub(crate) fn legacy(
        &mut self,
        actor: ObjId,
        at_utc: i64,
        known: &KnownDoc,
    ) -> Result<Option<mush_core::ModuleWrite>, ParticipantError> {
        if !self.enabled {
            return Ok(None);
        }
        check_known(&known.places)?;
        let event = Mutation {
            format: 1,
            actor,
            at_utc,
            source: Source::Legacy,
            known: known.clone(),
        };
        let append = self
            .ledger
            .prepare_append_bounded(LOG, event.value()?, RESIDENT_BYTES)
            .map_err(|_| refused())?;
        self.ledger
            .apply_prepared(append)
            .map(|(_, write)| Some(write))
            .map_err(|_| refused())
    }
}

pub(crate) fn refused() -> ParticipantError {
    ParticipantError::refused("Atlas movement knowledge is unavailable or changed")
}

pub(crate) fn check_known(places: &[RecordKey]) -> Result<(), ParticipantError> {
    if places.len() > MAX_KNOWN_PLACES {
        return Err(refused());
    }
    let mut bytes = 0_usize;
    for key in places {
        bytes = bytes
            .checked_add(key.pack.as_str().len())
            .and_then(|size| size.checked_add(key.entry.as_str().len()))
            .filter(|size| *size <= crate::movement::MAX_MOVEMENT_KEY_BYTES)
            .ok_or_else(refused)?;
    }
    Ok(())
}

pub(crate) fn canonical_known(places: &[RecordKey]) -> Result<(), ParticipantError> {
    check_known(places)?;
    if places.iter().collect::<BTreeSet<_>>().len() != places.len() {
        return Err(refused());
    }
    Ok(())
}

pub(crate) fn known_digest(
    places: Option<&[RecordKey]>,
) -> Result<DefinitionDigest, ParticipantError> {
    if let Some(places) = places {
        check_known(places)?;
    }
    DefinitionDigest::for_canonical_value_with_limit(
        "atlas.movement.known_projection.v1",
        &serde_json::to_value(places).map_err(|_| ParticipantError::Encoding)?,
        MAX_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}

/// Match the existing boot projection: first 512 entries, deduplicated in order.
pub(crate) fn normalized(doc: &KnownDoc) -> Vec<RecordKey> {
    let mut seen = BTreeSet::new();
    doc.places
        .iter()
        .take(MAX_KNOWN_PLACES)
        .filter(|key| seen.insert(*key))
        .cloned()
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Source {
    Legacy,
    Prepared {
        commit: CommitToken,
        prepared_at: ClockStamp,
        capture_digest: DefinitionDigest,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mutation {
    pub(crate) format: u16,
    pub(crate) actor: ObjId,
    pub(crate) at_utc: i64,
    pub(crate) source: Source,
    pub(crate) known: KnownDoc,
}
impl Mutation {
    pub(crate) fn value(&self) -> Result<Value, ParticipantError> {
        if self.format != 1 {
            return Err(refused());
        }
        check_known(&self.known.places)?;
        if let Source::Prepared { prepared_at, .. } = &self.source {
            canonical_known(&self.known.places)?;
            if !matches!(prepared_at.clock, mush_core::ClockRef::WorldIc { generation } if generation > 0)
            {
                return Err(refused());
            }
        }
        let value = serde_json::to_value(self).map_err(|_| ParticipantError::Encoding)?;
        mush_core::canonical_json_bytes_with_limit(&value, MAX_BYTES)
            .map_err(|_| ParticipantError::ImageLimit)?;
        Ok(value)
    }
    pub(crate) fn decode(value: &Value) -> Result<Self, ParticipantError> {
        mush_core::canonical_json_bytes_with_limit(value, MAX_BYTES)
            .map_err(|_| ParticipantError::ImageLimit)?;
        let row: Self = serde_json::from_value(value.clone()).map_err(|_| refused())?;
        if row.value()? != *value {
            return Err(refused());
        }
        Ok(row)
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        DefinitionDigest::for_canonical_value_with_limit(
            "atlas.movement.knowledge_event.v1",
            &self.value()?,
            MAX_BYTES,
        )
        .map_err(|_| ParticipantError::ImageLimit)
    }
}

#[cfg(test)]
#[path = "movement_history_tests.rs"]
mod tests;
