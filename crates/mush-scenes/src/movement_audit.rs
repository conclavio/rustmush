//! Read-only cold-boot join from exact joint receipts to original transcript
//! positions, including closed scenes. No callback, append, or current-scene
//! reconstruction participates in recovery.

use std::collections::BTreeMap;

use mush_core::action_state::{MAX_ACTION_STATE_BOOT_BYTES, MAX_ACTION_STATE_BOOT_RECORDS};
use mush_core::ledger::{LedgerDoc, parse_chunk, parse_chunk_key};
use mush_core::locomotion::composed::ComposedOwnerReader;
use mush_core::locomotion::scene_transition::{self, SceneLedgerAppendRef, SceneLocomotionReader};
use mush_core::{DefinitionDigest, ModError, ModuleBootData, ModuleCtx, ObjId};
use serde_json::{Value, json};

use crate::movement::{MAX_MOVEMENT_CAPTURE_BYTES, MovementCapture};

fn bad(message: &'static str) -> ModError {
    ModError::msg(message)
}

struct Route {
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
}
struct Proof {
    route: Route,
    digest: DefinitionDigest,
    events: Vec<(u64, Option<Vec<u8>>)>,
}
struct Expected {
    proof: usize,
    event: usize,
    digest: DefinitionDigest,
}
struct Audit {
    proofs: Vec<Proof>,
    expected: BTreeMap<(u64, u64), Expected>,
    remaining: usize,
}
impl Audit {
    fn new() -> Self {
        Self {
            proofs: Vec::new(),
            expected: BTreeMap::new(),
            remaining: MAX_ACTION_STATE_BOOT_BYTES,
        }
    }
    fn charge(&mut self, bytes: usize) -> Result<(), ModError> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or_else(|| bad("Scenes movement boot proof exceeds its memory bound"))?;
        Ok(())
    }
    fn add(
        &mut self,
        route: Route,
        digest: DefinitionDigest,
        appends: &[SceneLedgerAppendRef],
    ) -> Result<(), ModError> {
        if self.proofs.len() >= MAX_ACTION_STATE_BOOT_RECORDS || appends.len() > 2 {
            return Err(bad("Scenes movement boot proof exceeds its record bound"));
        }
        // Conservative per-row/index accounting includes Vec growth and tree
        // node overhead, before allocating any retained proof or index entry.
        self.charge(512 + appends.len() * 256)?;
        let proof = self.proofs.len();
        let mut events = Vec::with_capacity(appends.len());
        for (event, append) in appends.iter().enumerate() {
            if self
                .expected
                .insert(
                    (append.scene, append.sequence),
                    Expected {
                        proof,
                        event,
                        digest: append.event_digest,
                    },
                )
                .is_some()
            {
                return Err(bad(
                    "Scenes movement receipts claim the same transcript position",
                ));
            }
            events.push((append.scene, None));
        }
        self.proofs.push(Proof {
            route,
            digest,
            events,
        });
        Ok(())
    }

    fn read_history(&mut self, docs: &[LedgerDoc]) -> Result<(), ModError> {
        for (key, text) in docs {
            let Some((id, _)) = parse_chunk_key(key) else {
                return Err(bad("Scenes movement history has an invalid ledger key"));
            };
            let Some(scene) = id
                .parse::<u64>()
                .ok()
                .filter(|scene| scene.to_string() == id)
            else {
                continue;
            };
            if self
                .expected
                .range((scene, 0)..=(scene, u64::MAX))
                .next()
                .is_none()
            {
                continue;
            }
            let chunk = parse_chunk(text)
                .map_err(|_| bad("Scenes movement history cannot decode its ledger chunk"))?;
            for entry in chunk.entries {
                let Some(expected) = self.expected.remove(&(scene, entry.seq)) else {
                    continue;
                };
                let bytes = mush_core::canonical_json_bytes_with_limit(
                    &entry.value,
                    MAX_MOVEMENT_CAPTURE_BYTES,
                )
                .map_err(|_| bad("Scenes movement event exceeds its canonical bound"))?;
                self.charge(bytes.capacity())?;
                if scene_transition::scene_event_digest(&entry.value)
                    .map_err(|_| bad("Scenes movement event cannot be hashed"))?
                    != expected.digest
                {
                    return Err(bad(
                        "Scenes movement event differs from its committed receipt",
                    ));
                }
                let target = self
                    .proofs
                    .get_mut(expected.proof)
                    .and_then(|proof| proof.events.get_mut(expected.event))
                    .ok_or_else(|| bad("Scenes movement boot proof index is inconsistent"))?;
                if target.1.replace(bytes).is_some() {
                    return Err(bad("Scenes movement history repeats a committed event"));
                }
            }
        }
        if !self.expected.is_empty() {
            return Err(bad(
                "Scenes movement receipt has no original transcript event",
            ));
        }
        Ok(())
    }

    fn finish(self) -> Result<(), ModError> {
        for proof in self.proofs {
            let mut events = Vec::with_capacity(proof.events.len());
            for (scene, bytes) in proof.events {
                let bytes = bytes.ok_or_else(|| bad("Scenes movement event is missing"))?;
                let event: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| bad("Scenes movement event cannot decode"))?;
                events.push(json!({"scene":scene,"event":event}));
            }
            let capture = json!({"format":1,"actor":proof.route.actor,"from":proof.route.from,
                "to":proof.route.to,"at":proof.route.at,"events":events});
            let bytes =
                mush_core::canonical_json_bytes_with_limit(&capture, MAX_MOVEMENT_CAPTURE_BYTES)
                    .map_err(|_| bad("Scenes movement capture exceeds its bound"))?;
            MovementCapture::decode(&bytes)
                .map_err(|_| bad("Scenes movement capture is not canonical"))?;
            if scene_transition::scene_capture_digest(&capture)
                .map_err(|_| bad("Scenes movement capture cannot be hashed"))?
                != proof.digest
            {
                return Err(bad(
                    "Scenes movement capture differs from its committed receipt",
                ));
            }
        }
        Ok(())
    }
    fn check_empty_captures(&self) -> Result<(), ModError> {
        for proof in self.proofs.iter().filter(|proof| proof.events.is_empty()) {
            let capture = json!({"format":1,"actor":proof.route.actor,"from":proof.route.from,
                "to":proof.route.to,"at":proof.route.at,"events":[]});
            if scene_transition::scene_capture_digest(&capture)
                .map_err(|_| bad("Scenes empty movement capture cannot be hashed"))?
                != proof.digest
            {
                return Err(bad(
                    "Scenes empty movement capture differs from its receipt",
                ));
            }
        }
        Ok(())
    }
}

fn collect_receipts(
    ctx: &ModuleCtx<'_>,
    reader: Option<&SceneLocomotionReader>,
    composed: Option<&ComposedOwnerReader>,
) -> Result<Audit, ModError> {
    let mut audit = Audit::new();
    if let Some(reader) = reader {
        let mut after = None;
        loop {
            let page = reader
                .scan_receipts(ctx, after.as_deref(), 128)
                .map_err(|_| bad("Scenes movement receipts are unavailable or invalid"))?;
            for receipt in page.entries() {
                audit.add(
                    Route {
                        actor: receipt.actor(),
                        from: receipt.source(),
                        to: receipt.destination(),
                        at: receipt.at_utc(),
                    },
                    *receipt.scenes_capture_digest(),
                    receipt.appends(),
                )?;
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
    }
    if let Some(reader) = composed {
        let mut after = None;
        loop {
            let page = reader
                .scan_receipts(ctx, after.as_deref(), 128)
                .map_err(|_| bad("Scenes composed movement receipts are unavailable or invalid"))?;
            for receipt in page.entries() {
                let appends = crate::movement_composed::appends(receipt)?;
                audit.add(
                    Route {
                        actor: receipt.actor(),
                        from: receipt.source(),
                        to: receipt.destination(),
                        at: receipt.at_utc(),
                    },
                    *receipt.effect().owner_capture_digest(),
                    &appends,
                )?;
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
    }
    audit.check_empty_captures()?;
    Ok(audit)
}

/// Validate actual retained proof grammar, unique claimed positions and empty
/// capture digests without loading or reconstructing any live scene.
pub(crate) fn audit_typed(
    ctx: &ModuleCtx<'_>,
    reader: Option<&SceneLocomotionReader>,
    composed: Option<&ComposedOwnerReader>,
) -> Result<(), ModError> {
    collect_receipts(ctx, reader, composed).map(|_| ())
}

pub(crate) fn audit_boot(
    ctx: &ModuleCtx<'_>,
    data: &ModuleBootData,
    history: &mush_core::LedgerHistory,
    reader: Option<&SceneLocomotionReader>,
    composed: Option<&ComposedOwnerReader>,
) -> Result<(), ModError> {
    if !data.omitted.is_empty() {
        return Err(ModError::RetryFullFetch {
            module: crate::NS.into(),
            reason: "Scenes movement audit requires the complete transcript history".into(),
        });
    }
    // Native transcript validation remains a separate mandatory join.
    if !history.is_clean() {
        return Err(bad("Scenes movement history validation failed"));
    }
    let mut audit = collect_receipts(ctx, reader, composed)?;
    audit.read_history(&data.ledger)?;
    audit.finish()
}

#[cfg(test)]
#[path = "movement_audit_tests.rs"]
mod tests;
