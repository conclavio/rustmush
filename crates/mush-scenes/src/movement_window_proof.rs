//! Exact ordered proof grammar. Only scoped genuine Core receipts enter audit.

use std::collections::BTreeMap;

use mush_core::ledger::PreparedLedgerSequence;
use mush_core::locomotion::composed::ComposedOwnerReceipt;
use mush_core::locomotion::scene_transition::{self, SceneLedgerAppendRef};
use mush_core::{DefinitionRegistryBuilder, ModError, OwnerId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{bad, refused};
use crate::movement_window_capture::WindowSceneCapture;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    format: u16,
    leg_counts: Vec<u8>,
    appends: Vec<SceneLedgerAppendRef>,
}
pub(crate) struct OrderedClaims {
    pub(crate) leg_counts: Vec<u8>,
    pub(crate) appends: Vec<SceneLedgerAppendRef>,
}

pub(super) fn make(
    capture: &WindowSceneCapture,
    sequence: &PreparedLedgerSequence,
) -> Result<Value, mush_core::participant::ParticipantError> {
    let mut positions = sequence.positions();
    let mut appends = Vec::new();
    for event in capture.events() {
        let (id, sequence) = positions.next().ok_or_else(refused)?;
        if id != event.scene().to_string() {
            return Err(refused());
        }
        let value = serde_json::to_value(event.event()).map_err(|_| refused())?;
        appends.push(SceneLedgerAppendRef {
            scene: event.scene().0,
            sequence,
            event_digest: scene_transition::scene_event_digest(&value)?,
        });
    }
    if positions.next().is_some() {
        return Err(refused());
    }
    let leg_counts = capture
        .legs()
        .iter()
        .map(|leg| u8::try_from(leg.events().len()).map_err(|_| refused()))
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_value(Proof {
        format: 2,
        leg_counts,
        appends,
    })
    .map_err(|_| refused())
}

pub(crate) fn ordered_claims(
    receipt: &ComposedOwnerReceipt,
) -> Result<Option<OrderedClaims>, ModError> {
    let expected = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(bad)?,
            super::participant_descriptor()?,
        )
        .map_err(bad)?;
    if receipt.effect().participant() != &expected {
        return Ok(None);
    }
    let projection = receipt
        .window_projection()
        .ok_or_else(|| bad("ordered Scenes proof lacks its window header"))?;
    decode_claims(
        receipt.effect().proof(),
        receipt.source(),
        receipt.destination(),
        projection.projected_room(),
    )
    .map(Some)
}

pub(super) fn decode_claims(
    value: &Value,
    source: Option<mush_core::ObjId>,
    destination: mush_core::ObjId,
    projected: mush_core::ObjId,
) -> Result<OrderedClaims, ModError> {
    mush_core::canonical_json_bytes_with_limit(value, 4096).map_err(bad)?;
    let proof: Proof = serde_json::from_value(value.clone()).map_err(bad)?;
    let legs = if projected == destination { 1 } else { 2 };
    if proof.format != 2
        || proof.leg_counts.len() != legs
        || proof.leg_counts.iter().any(|n| *n > 2)
        || proof.appends.len() > 4
        || proof
            .leg_counts
            .iter()
            .map(|n| usize::from(*n))
            .sum::<usize>()
            != proof.appends.len()
        || (source == Some(destination) && proof.leg_counts.first() != Some(&0))
        || (source.is_none() && proof.leg_counts.first().is_none_or(|n| *n > 1))
        || serde_json::to_value(&proof).map_err(bad)? != *value
    {
        return Err(bad("invalid ordered Scenes proof"));
    }
    let mut offset = 0;
    for count in &proof.leg_counts {
        let end = offset + usize::from(*count);
        let rows = proof
            .appends
            .get(offset..end)
            .ok_or_else(|| bad("invalid ordered Scenes leg"))?;
        if let [first, second] = rows
            && first.scene == second.scene
        {
            return Err(bad("ordered Scenes leg repeats one transcript"));
        }
        offset = end;
    }
    let mut last = BTreeMap::<u64, u64>::new();
    for append in &proof.appends {
        if append.event_digest.as_bytes() == &[0; 32]
            || last
                .insert(append.scene, append.sequence)
                .is_some_and(|previous| previous.checked_add(1) != Some(append.sequence))
        {
            return Err(bad(
                "ordered Scenes positions are not original consecutive appends",
            ));
        }
    }
    Ok(OrderedClaims {
        leg_counts: proof.leg_counts,
        appends: proof.appends,
    })
}
