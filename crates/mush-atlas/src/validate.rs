//! The validator's S1 slice (doc 14 §4.3): decode every entry of a pack into its
//! authored shape and report what does not parse, before anything is applied.
//! The geometric refusals — overlaps, an exit whose rooms do not touch — need the
//! world's records and are raised by the appliers at apply time; S3 lifts them into
//! the agent's validator tool.

use mush_content::{Appliers, Pack};

use crate::def::{
    CheckpointDef, ConnectionDef, ExitDef, FeatureDef, HookDef, ItemDef, LocalityDef,
    NeighborhoodDef, RegionDef, RoomDef, SecretDef, SiteDef, StructureDef,
};

/// Every problem a dry decode finds, one line each. Empty is clean.
#[must_use]
pub fn check_pack(pack: &Pack, appliers: &Appliers) -> Vec<String> {
    let mut problems = Vec::new();
    let mut neighborhoods = 0_usize;
    let mut regions = 0_usize;
    for (id, entry) in pack.entries() {
        let kind = entry.kind().as_str();
        if appliers.kinds().owner_of(entry.kind()).is_none() {
            problems.push(format!("{id}: nothing on this server owns kind {kind}"));
            continue;
        }
        let outcome: Result<(), String> = match kind {
            "region" => {
                regions += 1;
                entry
                    .decode::<RegionDef>()
                    .map_err(|e| e.to_string())
                    .and_then(|d| d.region_kind().map(|_| ()))
            }
            "locality" => entry
                .decode::<LocalityDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "checkpoint" => entry
                .decode::<CheckpointDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "connection" => entry
                .decode::<ConnectionDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "neighborhood" => {
                neighborhoods += 1;
                entry
                    .decode::<NeighborhoodDef>()
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            "site" => entry
                .decode::<SiteDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "structure" => entry
                .decode::<StructureDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "room" => entry
                .decode::<RoomDef>()
                .and_then(|d| {
                    d.footprint.to_footprint().shape().map(|_| ()).map_err(|e| {
                        mush_content::PackError::EntryDecode {
                            kind: "room".to_string(),
                            detail: e.to_string(),
                        }
                    })
                })
                .map_err(|e| e.to_string()),
            "feature" => entry
                .decode::<FeatureDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "item" => entry
                .decode::<ItemDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "exit" => entry
                .decode::<ExitDef>()
                .and_then(|d| {
                    if d.names.is_empty() || d.names.len() > 2 {
                        Err(mush_content::PackError::EntryDecode {
                            kind: "exit".to_string(),
                            detail: "an exit row names one exit, or two".to_string(),
                        })
                    } else {
                        Ok(())
                    }
                })
                .map_err(|e| e.to_string()),
            "hook" => entry
                .decode::<HookDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "secret" => entry
                .decode::<SecretDef>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            // Another owner's kind: it decodes at apply.
            _ => Ok(()),
        };
        if let Err(problem) = outcome {
            problems.push(format!("{id}: {problem}"));
        }
    }
    let kind = pack.manifest().kind();
    if kind == mush_content::PackKind::Neighborhood && neighborhoods != 1 {
        problems.push(format!(
            "a neighborhood pack has exactly one neighborhood entry; this one has {neighborhoods}"
        ));
    }
    if kind == mush_content::PackKind::Region && regions != 1 {
        problems.push(format!(
            "a region pack has exactly one region entry; this one has {regions}"
        ));
    }
    problems
}
