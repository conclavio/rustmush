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
    window: Option<(ObjId, Vec<u8>)>,
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
        if appends.len() > 2 {
            return Err(bad("Scenes movement boot proof exceeds its record bound"));
        }
        self.add_profile(route, None, digest, appends)
    }
    fn add_window(
        &mut self,
        route: Route,
        projected: ObjId,
        counts: Vec<u8>,
        digest: DefinitionDigest,
        appends: &[SceneLedgerAppendRef],
    ) -> Result<(), ModError> {
        if counts.len() > 2
            || counts.iter().any(|n| *n > 2)
            || counts.iter().map(|n| usize::from(*n)).sum::<usize>() != appends.len()
        {
            return Err(bad("Scenes ordered leg counts exceed their bound"));
        }
        self.add_profile(route, Some((projected, counts)), digest, appends)
    }
    fn add_profile(
        &mut self,
        route: Route,
        window: Option<(ObjId, Vec<u8>)>,
        digest: DefinitionDigest,
        appends: &[SceneLedgerAppendRef],
    ) -> Result<(), ModError> {
        if self.proofs.len() >= MAX_ACTION_STATE_BOOT_RECORDS || appends.len() > 4 {
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
            window,
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
            let Proof {
                route,
                window,
                digest,
                events: original,
            } = proof;
            let mut events = Vec::with_capacity(original.len());
            for (scene, bytes) in original {
                let bytes = bytes.ok_or_else(|| bad("Scenes movement event is missing"))?;
                let event: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| bad("Scenes movement event cannot decode"))?;
                events.push(json!({"scene":scene,"event":event}));
            }
            verify_capture(&route, window.as_ref(), events, digest)?;
        }
        Ok(())
    }
    fn check_empty_captures(&self) -> Result<(), ModError> {
        for proof in self.proofs.iter().filter(|proof| proof.events.is_empty()) {
            verify_capture(&proof.route, proof.window.as_ref(), vec![], proof.digest)?;
        }
        Ok(())
    }
}

fn verify_capture(
    route: &Route,
    window: Option<&(ObjId, Vec<u8>)>,
    events: Vec<Value>,
    expected: DefinitionDigest,
) -> Result<(), ModError> {
    let digest = if let Some((projected, counts)) = window {
        let mut remaining = events.into_iter();
        let mut legs = Vec::with_capacity(counts.len());
        for (index, count) in counts.iter().enumerate() {
            let events: Vec<_> = remaining.by_ref().take(usize::from(*count)).collect();
            if events.len() != usize::from(*count) {
                return Err(bad("Scenes ordered leg is missing events"));
            }
            let (from, to) = if index == 0 {
                (route.from, route.to)
            } else {
                (Some(route.to), *projected)
            };
            legs.push(json!({"format":1,"actor":route.actor,"from":from,"to":to,"at":route.at,"events":events}));
        }
        if remaining.next().is_some() {
            return Err(bad("Scenes ordered proof has extra events"));
        }
        let capture = json!({"format":1,"actor":route.actor,"source":route.from,"admitted_room":route.to,
            "projected_room":projected,"at":route.at,"legs":legs});
        let bytes = mush_core::canonical_json_bytes_with_limit(
            &capture,
            crate::movement_window_capture::MAX_WINDOW_SCENE_CAPTURE_BYTES,
        )
        .map_err(|_| bad("Scenes ordered capture exceeds its bound"))?;
        crate::movement_window_capture::WindowSceneCapture::decode(&bytes)
            .map_err(|_| bad("Scenes ordered capture is not canonical"))?
            .digest()
            .map_err(|_| bad("Scenes ordered capture cannot be hashed"))?
    } else {
        let capture = json!({"format":1,"actor":route.actor,"from":route.from,"to":route.to,"at":route.at,"events":events});
        let bytes =
            mush_core::canonical_json_bytes_with_limit(&capture, MAX_MOVEMENT_CAPTURE_BYTES)
                .map_err(|_| bad("Scenes movement capture exceeds its bound"))?;
        MovementCapture::decode(&bytes)
            .map_err(|_| bad("Scenes movement capture is not canonical"))?;
        scene_transition::scene_capture_digest(&capture)
            .map_err(|_| bad("Scenes movement capture cannot be hashed"))?
    };
    if digest != expected {
        return Err(bad(
            "Scenes movement capture differs from its committed receipt",
        ));
    }
    Ok(())
}

fn collect_receipts<'a>(
    ctx: &ModuleCtx<'_>,
    reader: Option<&SceneLocomotionReader>,
    composed: impl IntoIterator<Item = &'a ComposedOwnerReader>,
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
    for reader in composed {
        let mut after = None;
        loop {
            let page = reader
                .scan_receipts(ctx, after.as_deref(), 128)
                .map_err(|_| bad("Scenes composed movement receipts are unavailable or invalid"))?;
            for receipt in page.entries() {
                let route = Route {
                    actor: receipt.actor(),
                    from: receipt.source(),
                    to: receipt.destination(),
                    at: receipt.at_utc(),
                };
                let digest = *receipt.effect().owner_capture_digest();
                if let Some(claims) = crate::movement_window::ordered_claims(receipt)? {
                    let projected = receipt
                        .window_projection()
                        .ok_or_else(|| bad("Scenes window header is missing"))?
                        .projected_room();
                    audit.add_window(
                        route,
                        projected,
                        claims.leg_counts,
                        digest,
                        &claims.appends,
                    )?;
                } else {
                    audit.add(route, digest, &crate::movement_composed::appends(receipt)?)?;
                }
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
pub(crate) fn audit_typed<'a>(
    ctx: &ModuleCtx<'_>,
    reader: Option<&SceneLocomotionReader>,
    composed: impl IntoIterator<Item = &'a ComposedOwnerReader>,
) -> Result<(), ModError> {
    collect_receipts(ctx, reader, composed).map(|_| ())
}

pub(crate) fn audit_boot<'a>(
    ctx: &ModuleCtx<'_>,
    data: &ModuleBootData,
    history: &mush_core::LedgerHistory,
    reader: Option<&SceneLocomotionReader>,
    composed: impl IntoIterator<Item = &'a ComposedOwnerReader>,
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
