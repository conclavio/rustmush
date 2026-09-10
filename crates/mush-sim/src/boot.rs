//! Boot: rebuild [`SpatialState`] from the `sim` docs, validate every record
//! against the graph invariants, quarantine what fails, rebuild the indexes,
//! and reconcile projections (design §9.4, §13; cut-1 review).
//!
//! Policy: a corrupt `meta` or container doc (domain/space/mounts) FAILS the
//! boot — losing a container would strand everything under it (the comsys /
//! scenes precedent). A structurally valid but dangling placement is
//! quarantined to the recovery room. An unreadable placement is instead held
//! mechanically inactive in RAM while its authoritative document remains
//! byte-for-byte untouched for a later repair workflow.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{DefinitionRef, Flags, ModError, ModuleBootData, ModuleCtx, ObjId, RulesValue};

use crate::data::{
    DomainRecord, DomainStatus, HeldPlacement, HostAssignments, HostMounts, HostStations,
    InteriorSpaceRecord, META_KEY, PASSIVE_DRIVER, PlacementRecord, SpaceAnchor, SpatialParent,
    SpatialState, UnplacedInfo, checked_placement_revision, passive_definition_ref,
    passive_rules_value, persist_assignments, persist_del, persist_placement, persist_space,
    placement_key,
};
use crate::graph::{ancestors, project, rebuild};
use crate::station::{StationFallback, StationFallbacks, fallback_key};

fn parse_id(key: &str, prefix: &str) -> Option<u64> {
    key.strip_prefix(prefix)?.parse().ok()
}

fn bounded_hold_reason(reason: impl AsRef<str>) -> String {
    const MAX_CHARS: usize = 512;
    let reason = reason.as_ref();
    let mut chars = reason.chars();
    let mut bounded: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        bounded.push('…');
    }
    bounded
}

/// Upgrade the one legacy rules-value identity this module owns. This adapter
/// is intentionally record-contextual: it neither makes shared serde lenient
/// nor guesses an owner by splitting arbitrary dotted schema names.
fn upgrade_legacy_passive(value: &mut serde_json::Value) -> Result<(), String> {
    let Some(fields) = value.as_object() else {
        return Ok(());
    };
    let Some(schema) = fields.get("schema").and_then(serde_json::Value::as_str) else {
        return Ok(());
    };
    if fields.len() != 3 || !fields.contains_key("version") || !fields.contains_key("data") {
        return Err("legacy rules value has unknown or missing fields".to_string());
    }
    if schema != PASSIVE_DRIVER {
        return Err(format!("unsupported legacy rules schema {schema:?}"));
    }
    match fields.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) => {}
        Some(version) => {
            return Err(format!(
                "unsupported legacy rules schema {PASSIVE_DRIVER:?} version {version}"
            ));
        }
        None => return Err("legacy rules value has no integer version".to_string()),
    }
    let data = fields
        .get("data")
        .cloned()
        .ok_or_else(|| "legacy rules value has no data".to_string())?;
    let upgraded = passive_rules_value(data)
        .map_err(|e| format!("legacy passive rules value is invalid: {e}"))?;
    *value = serde_json::to_value(upgraded)
        .map_err(|e| format!("could not encode upgraded passive rules value: {e}"))?;
    Ok(())
}

/// Explicit compatibility adapter for the one legacy domain-driver tuple
/// owned by this module.  An exact v2 driver is left untouched; no dotted id
/// is split or resolved generically.
fn upgrade_domain_doc(value: &mut serde_json::Value) -> Result<(), String> {
    let Some(fields) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(driver) = fields.get("driver") else {
        return Ok(());
    };
    if driver.is_object() {
        if fields.contains_key("schema_version") {
            return Err("exact domain driver also carries legacy schema_version".to_string());
        }
        return Ok(());
    }
    let Some(driver) = driver.as_str() else {
        return Err("legacy domain driver is not a string".to_string());
    };
    if driver != PASSIVE_DRIVER {
        return Err(format!("unsupported legacy domain driver {driver:?}"));
    }
    match fields
        .remove("schema_version")
        .and_then(|version| version.as_u64())
    {
        Some(1) => {}
        Some(version) => {
            return Err(format!(
                "unsupported legacy domain driver {PASSIVE_DRIVER:?} version {version}"
            ));
        }
        None => return Err("legacy domain driver has no integer schema_version".to_string()),
    }
    fields.insert(
        "driver".to_string(),
        serde_json::to_value(passive_definition_ref())
            .map_err(|e| format!("could not encode passive driver definition: {e}"))?,
    );
    Ok(())
}

fn validate_exact_passive(
    ctx: &ModuleCtx<'_>,
    reference: &DefinitionRef,
    context: &str,
) -> Result<(), String> {
    let definitions = ctx
        .reg
        .definitions()
        .ok_or_else(|| "definition registry is not frozen".to_string())?;
    definitions
        .get_exact(reference)
        .map_err(|e| format!("{context} refers to unavailable definition: {e}"))?;
    let expected = passive_definition_ref();
    if reference != &expected {
        return Err(format!(
            "{context} uses unsupported definition {reference}; expected {expected}"
        ));
    }
    Ok(())
}

fn validate_rules_value(
    ctx: &ModuleCtx<'_>,
    value: &RulesValue,
    context: &str,
) -> Result<(), String> {
    validate_exact_passive(ctx, value.schema(), context)
}

fn validate_parent_refs(
    ctx: &ModuleCtx<'_>,
    parent: &SpatialParent,
    context: &str,
) -> Result<(), String> {
    match parent {
        SpatialParent::Direct { pose, .. } => validate_rules_value(ctx, &pose.posture, context),
        SpatialParent::Attached {
            local_pose: Some(pose),
            ..
        } => validate_rules_value(ctx, pose, context),
        SpatialParent::Interior { .. }
        | SpatialParent::Attached {
            local_pose: None, ..
        }
        | SpatialParent::Unplaced => Ok(()),
    }
}

fn validate_placement_refs(ctx: &ModuleCtx<'_>, placement: &PlacementRecord) -> Result<(), String> {
    validate_parent_refs(ctx, &placement.parent, "placement pose")?;
    if let Some(previous) = placement
        .unplaced
        .as_ref()
        .and_then(|unplaced| unplaced.previous.as_ref())
    {
        validate_parent_refs(ctx, previous, "unplaced previous pose")?;
    }
    Ok(())
}

fn validate_space_refs(ctx: &ModuleCtx<'_>, space: &InteriorSpaceRecord) -> Result<(), String> {
    if let SpaceAnchor::FixedDomain {
        anchor_pose: Some(pose),
        ..
    } = &space.anchor
    {
        validate_rules_value(ctx, pose, "space anchor pose")?;
    }
    for egress in space.egresses.values() {
        if let Some(pose) = &egress.to_pose {
            validate_rules_value(ctx, pose, "egress pose")?;
        }
    }
    Ok(())
}

fn validate_station_refs(ctx: &ModuleCtx<'_>, stations: &HostStations) -> Result<(), String> {
    for station in stations.stations.values() {
        validate_rules_value(ctx, &station.policy, "station policy")?;
    }
    Ok(())
}

fn upgrade_spatial_parent(value: &mut serde_json::Value) -> Result<(), String> {
    let Some(fields) = value.as_object_mut() else {
        return Ok(());
    };
    match fields.get("kind").and_then(serde_json::Value::as_str) {
        Some("Direct") => {
            // S4: the pose is metric with the rules value as its posture; a
            // cut-1 document carries the bare rules value, which still decodes.
            if let Some(pose) = fields.get_mut("pose") {
                match pose.get_mut("posture") {
                    Some(posture) => upgrade_legacy_passive(posture)?,
                    None => upgrade_legacy_passive(pose)?,
                }
            }
        }
        Some("Attached") => {
            if let Some(local_pose) = fields.get_mut("local_pose") {
                upgrade_legacy_passive(local_pose)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn upgrade_placement_doc(value: &mut serde_json::Value) -> Result<(), String> {
    if let Some(parent) = value.get_mut("parent") {
        upgrade_spatial_parent(parent)?;
    }
    if let Some(previous) = value
        .get_mut("unplaced")
        .and_then(|unplaced| unplaced.get_mut("previous"))
    {
        upgrade_spatial_parent(previous)?;
    }
    Ok(())
}

fn upgrade_space_doc(value: &mut serde_json::Value) -> Result<(), String> {
    if let Some(anchor) = value.get_mut("anchor")
        && anchor.get("kind").and_then(serde_json::Value::as_str) == Some("FixedDomain")
        && let Some(anchor_pose) = anchor.get_mut("anchor_pose")
    {
        upgrade_legacy_passive(anchor_pose)?;
    }
    if let Some(egresses) = value
        .get_mut("egresses")
        .and_then(serde_json::Value::as_object_mut)
    {
        for egress in egresses.values_mut() {
            if let Some(to_pose) = egress.get_mut("to_pose") {
                upgrade_legacy_passive(to_pose)?;
            }
        }
    }
    Ok(())
}

fn upgrade_stations_doc(value: &mut serde_json::Value) -> Result<(), String> {
    if let Some(stations) = value
        .get_mut("stations")
        .and_then(serde_json::Value::as_object_mut)
    {
        for station in stations.values_mut() {
            if let Some(policy) = station.get_mut("policy") {
                upgrade_legacy_passive(policy)?;
            }
        }
    }
    Ok(())
}

/// Rebuild and install the state from boot data. Returns the number of
/// placements quarantined.
pub(crate) fn load_boot(
    ctx: &mut ModuleCtx<'_>,
    data: ModuleBootData,
    recovery_room: ObjId,
) -> Result<usize, ModError> {
    let mut st = SpatialState::new(recovery_room);
    let mut meta_next: Option<u64> = None;
    let mut placements: BTreeMap<ObjId, PlacementRecord> = BTreeMap::new();
    let mut raw_assignments: Vec<HostAssignments> = Vec::new();
    let mut station_fallbacks: Vec<StationFallback> = Vec::new();
    let mut station_services: Vec<crate::seat::StationService> = Vec::new();
    // J1: the recovery ledger's chunks arrive as text in `data.ledger`.
    let log_docs = data.ledger;

    for (key, value) in data.kv {
        if key == META_KEY {
            // A present-but-corrupt meta doc fails the boot: silently guessing
            // the allocator could reissue a live domain id.
            meta_next = Some(
                value
                    .get("next_domain_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| ModError::msg("sim: meta doc is corrupt (no next_domain_id)"))?,
            );
        } else if key.starts_with("domain:") {
            let mut value = value;
            upgrade_domain_doc(&mut value)
                .map_err(|e| ModError::msg(format!("sim: bad domain doc {key:?}: {e}")))?;
            let d: DomainRecord = serde_json::from_value(value)
                .map_err(|e| ModError::msg(format!("sim: bad domain doc {key:?}: {e}")))?;
            validate_exact_passive(ctx, &d.driver, "domain driver")
                .map_err(|e| ModError::msg(format!("sim: bad domain doc {key:?}: {e}")))?;
            if parse_id(&key, "domain:") != Some(d.id.0) {
                return Err(ModError::msg(format!(
                    "sim: doc {key:?} claims domain #{}",
                    d.id
                )));
            }
            st.domains.insert(d.id, d);
        } else if key.starts_with("space:") {
            let mut value = value;
            upgrade_space_doc(&mut value)
                .map_err(|e| ModError::msg(format!("sim: bad space doc {key:?}: {e}")))?;
            let s: InteriorSpaceRecord = serde_json::from_value(value)
                .map_err(|e| ModError::msg(format!("sim: bad space doc {key:?}: {e}")))?;
            validate_space_refs(ctx, &s)
                .map_err(|e| ModError::msg(format!("sim: bad space doc {key:?}: {e}")))?;
            if parse_id(&key, "space:") != Some(u64::from(s.object.0)) {
                return Err(ModError::msg(format!(
                    "sim: doc {key:?} claims space {}",
                    s.object
                )));
            }
            st.spaces.insert(s.object, s);
        } else if key.starts_with("mounts:") {
            let m: HostMounts = serde_json::from_value(value)
                .map_err(|e| ModError::msg(format!("sim: bad mounts doc {key:?}: {e}")))?;
            if parse_id(&key, "mounts:") != Some(u64::from(m.host.0)) {
                return Err(ModError::msg(format!(
                    "sim: doc {key:?} claims host {}",
                    m.host
                )));
            }
            st.mounts.insert(m.host, m);
        } else if key.starts_with("stations:") {
            // Container-like (a unit's control surface): a corrupt or
            // key/embedded-host-mismatched doc fails boot.
            let mut value = value;
            upgrade_stations_doc(&mut value)
                .map_err(|e| ModError::msg(format!("sim: bad stations doc {key:?}: {e}")))?;
            let hs: HostStations = serde_json::from_value(value)
                .map_err(|e| ModError::msg(format!("sim: bad stations doc {key:?}: {e}")))?;
            validate_station_refs(ctx, &hs)
                .map_err(|e| ModError::msg(format!("sim: bad stations doc {key:?}: {e}")))?;
            if parse_id(&key, "stations:") != Some(u64::from(hs.host.0)) {
                return Err(ModError::msg(format!(
                    "sim: doc {key:?} claims host {}",
                    hs.host
                )));
            }
            st.stations.insert(hs.host, hs);
        } else if key.starts_with("assignment:") {
            // Recoverable (a pilot re-boards): a corrupt doc is dropped, not fatal.
            match serde_json::from_value::<HostAssignments>(value) {
                // E7c10 B1 (10b §3.10 contradiction (vii)): the seat census and
                // every seat's narrowed channel set are re-run HERE, so a stored
                // document over `MAX_SEATS_PER_UNIT` / `MAX_GRANTED_CHANNELS` is
                // REFUSED on load and its host marked — never truncated into a
                // silently smaller crew.
                Ok(ha) if parse_id(&key, "assignment:") == Some(u64::from(ha.host.0)) => {
                    match ha.check() {
                        Ok(()) => raw_assignments.push(ha),
                        Err(error) => {
                            // B4: HELD, never merely logged. The document stays on
                            // disk and its host refuses every seat write, so the
                            // next ordinary `@spatial/assign` cannot rebuild a
                            // one-seat document over a 65-seat one staff have yet
                            // to repair.
                            tracing::error!(%key, host = %ha.host, %error, "sim: over-cap assignment doc refused on load and HELD");
                            let reason = bounded_hold_reason(&error);
                            st.held_assignments.insert(
                                ha.host,
                                crate::data::HeldPlacement {
                                    key: key.clone(),
                                    reason,
                                },
                            );
                        }
                    }
                }
                Ok(ha) => {
                    tracing::error!(%key, host = %ha.host, "sim: assignment doc key/host mismatch dropped")
                }
                Err(e) => {
                    tracing::error!(%key, error = %e, "sim: unparseable assignment doc dropped")
                }
            }
        } else if let Some(rest) = key.strip_prefix("cgen:") {
            // A unit's control epoch (monotonic across reboot). Unreadable = drop.
            match (rest.parse::<u32>(), value.as_u64()) {
                (Ok(n), Some(g)) => {
                    st.control_gen.insert(ObjId(n), g);
                }
                _ => tracing::error!(%key, "sim: unparseable control-gen doc dropped"),
            }
        } else if let Some(rest) = key.strip_prefix("placement:") {
            let Some(object) = rest
                .parse::<u32>()
                .ok()
                .map(ObjId)
                .filter(|object| rest == object.0.to_string())
            else {
                let reason = bounded_hold_reason("placement key has no canonical object id");
                tracing::error!(%key, %reason, "sim: unreadable authoritative placement held without activation");
                st.unbound_placement_docs.insert(key, reason);
                continue;
            };
            let mut value = value;
            let parsed = upgrade_placement_doc(&mut value).and_then(|()| {
                let placement =
                    serde_json::from_value::<PlacementRecord>(value).map_err(|e| e.to_string())?;
                if placement.object != object {
                    return Err(format!(
                        "placement key identifies {object}, but document claims {}",
                        placement.object
                    ));
                }
                validate_placement_refs(ctx, &placement)?;
                Ok(placement)
            });
            match parsed {
                Ok(p) => {
                    placements.insert(p.object, p);
                }
                Err(e) => {
                    let reason = bounded_hold_reason(e);
                    tracing::error!(%key, %reason, "sim: unreadable authoritative placement held without activation");
                    st.held_placements.insert(
                        object,
                        HeldPlacement {
                            key: key.clone(),
                            reason,
                        },
                    );
                }
            }
        } else if key.starts_with("station_fallback:") {
            // D2: the declared fallback controllers (sibling of `cgen:`). Without
            // this branch the docs fell to the "unknown doc ignored" warn every
            // boot and the RAM mirror stayed empty, so a durable staff policy went
            // silently inert after the routine reboot. Drop-on-unreadable (never
            // fatal): a bad fallback record must not brick boot.
            match serde_json::from_value::<StationFallback>(value) {
                Ok(rec) if fallback_key(rec.unit, &rec.station) == key => {
                    station_fallbacks.push(rec);
                }
                Ok(rec) => {
                    tracing::error!(%key, unit = %rec.unit, "sim: station-fallback doc key/body mismatch dropped")
                }
                Err(e) => {
                    tracing::error!(%key, error = %e, "sim: unparseable station-fallback doc dropped")
                }
            }
        } else if key.starts_with("station_service:") {
            // E7c10 B8: the applied markers `sim.station.served` answers from
            // (an EXEMPT workflow record, never authority). Drop-on-unreadable,
            // exactly like a fallback doc: a bad marker must not brick boot, and
            // a missing one answers `None` = unknown = unsatisfied, which is the
            // fail-closed direction for a fact-gated settlement.
            match serde_json::from_value::<crate::seat::StationService>(value) {
                Ok(rec) if crate::seat::service_key(rec.unit, &rec.station, rec.actor) == key => {
                    station_services.push(rec);
                }
                Ok(rec) => {
                    tracing::error!(%key, unit = %rec.unit, "sim: station-service doc key/body mismatch dropped")
                }
                Err(e) => {
                    tracing::error!(%key, error = %e, "sim: unparseable station-service doc dropped")
                }
            }
        } else {
            tracing::warn!(%key, "sim: unknown doc ignored");
        }
    }
    st.recovery
        .restore_from_kv(&log_docs)
        .map_err(|error| ModError::msg(format!("sim recovery ledger restore failed: {error}")))?;
    let discovered_next = match st.domains.keys().next_back() {
        Some(domain) => domain.0.checked_add(1).ok_or_else(|| {
            ModError::msg(format!(
                "sim: domain #{domain} exhausts the id space; no resume cursor is representable"
            ))
        })?,
        None => 0,
    };
    st.next_domain_id = meta_next.unwrap_or(0).max(discovered_next);

    // If duplicate input somehow supplied both readable and unreadable values
    // for one canonical key, the held posture wins: never activate ambiguous
    // state. Real stores provide unique keys, but boot data is still untrusted.
    placements.retain(|object, _| !st.held_placements.contains_key(object));

    // Validate placements against the world and the graph.
    st.placements = placements;
    st.derived = rebuild(&st);
    let now = ctx.w.now();

    // A readable record whose object no longer exists is a tombstone. Held
    // unreadable documents are intentionally excluded: K1 never rewrites or
    // deletes their authoritative keys before a repair workflow exists.
    let mut delete: BTreeSet<ObjId> = BTreeSet::new();
    // Failure roots are readable records that fail live validation. Each takes
    // its whole subtree so no valid child is left under an Unplaced parent.
    let mut roots: BTreeMap<ObjId, String> = BTreeMap::new();
    let ids: Vec<ObjId> = st.placements.keys().copied().collect();
    for obj in ids {
        let Some(rec) = st.placements.get(&obj) else {
            continue;
        };
        if !rec.parent.is_placed() {
            continue;
        }
        if ctx.w.get(obj).is_none() {
            delete.insert(obj);
            continue;
        }
        if let Some(reason) = validate_live(ctx, &st, &rec.clone()) {
            roots.insert(obj, format!("boot validation: {reason}"));
        }
    }
    let mut quarantine: BTreeMap<ObjId, String> = BTreeMap::new();
    for (root, reason) in &roots {
        for n in st.subtree(*root) {
            if delete.contains(&n) {
                continue;
            }
            quarantine.entry(n).or_insert_with(|| {
                if n == *root {
                    reason.clone()
                } else {
                    format!("boot: ancestor {root} quarantined")
                }
            });
        }
    }
    // Preflight every persisted quarantine revision before emitting deletes or
    // modifying the fresh boot state. A MAX revision needs an explicit repair;
    // boot must not silently publish a different record at the same revision.
    let mut quarantine_revisions: BTreeMap<ObjId, u64> = BTreeMap::new();
    for obj in quarantine.keys() {
        let revision = match st.placements.get(obj) {
            Some(previous) => checked_placement_revision(*obj, previous.revision)
                .map_err(|error| ModError::msg(format!("sim: boot quarantine refused: {error}")))?,
            None => 1,
        };
        quarantine_revisions.insert(*obj, revision);
    }
    for o in &delete {
        st.placements.remove(o);
        persist_del(ctx.out, placement_key(*o));
    }
    let quarantined = quarantine.len() + st.held_placements.len() + st.unbound_placement_docs.len();
    for (obj, reason) in &quarantine {
        let prev = st.placements.get(obj);
        let q = PlacementRecord {
            object: *obj,
            parent: SpatialParent::Unplaced,
            revision: quarantine_revisions[obj],
            unplaced: Some(UnplacedInfo {
                reason: reason.clone(),
                by: None,
                at: now,
                previous: prev.map(|p| p.parent.clone()),
                quarantined: true,
            }),
        };
        tracing::warn!(obj = %obj, reason = %reason, "sim: placement quarantined at boot");
        persist_placement(ctx.out, &q);
        st.placements.insert(*obj, q);
    }
    st.derived = rebuild(&st);
    // Spaces fixed in a retired/missing domain lose their anchor.
    let space_ids: Vec<ObjId> = st.spaces.keys().copied().collect();
    for sid in space_ids {
        let Some(s) = st.spaces.get(&sid) else {
            continue;
        };
        let dangling = match &s.anchor {
            SpaceAnchor::FixedDomain { domain, .. } => st
                .domains
                .get(domain)
                .is_none_or(|d| d.status == DomainStatus::Retired),
            SpaceAnchor::MovingObject { host } => ctx
                .w
                .get(*host)
                .is_none_or(|o| o.flags.contains(Flags::GOING)),
            SpaceAnchor::Unanchored => false,
        };
        if dangling {
            tracing::warn!(space = %sid, "sim: space anchor dangling; re-anchored to Unanchored");
            let mut rec = s.clone();
            rec.anchor = SpaceAnchor::Unanchored;
            persist_space(ctx.out, &rec);
            st.spaces.insert(sid, rec);
        }
    }
    st.derived = rebuild(&st);

    // Reconcile projections (idempotent, record-driven).
    let ids: Vec<ObjId> = st.placements.keys().copied().collect();
    for obj in ids {
        let Some(dest) = project(&st, ctx.w, obj) else {
            continue;
        };
        if ctx.w.get(obj).and_then(|o| o.location) == Some(dest) {
            continue;
        }
        if ctx.w.get(obj).is_none() {
            continue;
        }
        match ctx.w.move_to(obj, dest) {
            Ok(()) => {
                tracing::warn!(obj = %obj, dest = %dest, "sim: projection reconciled at boot")
            }
            Err(e) => {
                tracing::error!(obj = %obj, dest = %dest, error = %e, "sim: could not reconcile projection")
            }
        }
    }

    // A held record contributes no graph state, but a live associated object
    // remains visibly contained in the recovery room and movement-gated. This
    // is an in-memory/world projection only; the authoritative module key is
    // never emitted as a write or delete.
    let held_objects: Vec<ObjId> = st.held_placements.keys().copied().collect();
    let recovery_destination = if ctx
        .w
        .get(st.recovery_room)
        .is_some_and(|room| !room.flags.contains(Flags::GOING))
    {
        st.recovery_room
    } else {
        mush_core::LIMBO
    };
    for object in held_objects {
        if ctx
            .w
            .get(object)
            .is_none_or(|object| object.flags.contains(Flags::GOING))
            || ctx.w.get(object).and_then(|object| object.location) == Some(recovery_destination)
        {
            continue;
        }
        match ctx.w.move_to(object, recovery_destination) {
            Ok(()) => {
                tracing::warn!(obj = %object, "sim: held placement projected to recovery room without rewriting its raw document")
            }
            Err(error) => {
                tracing::error!(obj = %object, %error, "sim: could not project held placement to recovery room")
            }
        }
    }

    // Assignments: drop a seat whose actor is gone and drop a whole doc whose
    // host is gone; persist any doc we prune. Station DEFS (container-like) are
    // trusted as loaded.
    let live = |id: ObjId| {
        ctx.w
            .get(id)
            .is_some_and(|o| !o.flags.contains(Flags::GOING))
    };
    let mut assigned_actors: BTreeSet<ObjId> = BTreeSet::new();
    // **E7 cut 10 independent verify fold, A6 (decision E4(a)): no focus survives
    // a boot, so no NARROWING may.**
    //
    // `StationAssignment.granted` is persisted in the exempt `assignment:<unit>`
    // document and re-hydrated here, then republished through
    // `refresh_actor_links` -> `desired_links` -> `granted_channels(spec)`.
    // `FocusState` is deliberately never hydrated (D-E7-34) and `SeatAuditLog` is
    // process-local (deviation (d)), so cut 10 brought every live proxy seat's
    // station back **permanently narrowed with nobody driving it**: no focus, no
    // re-activatable grant (the audit row is gone, so `+seat/activate` pages and
    // refuses), and nothing anywhere reconciled the two — the withheld channels
    // were simply missing from the whole crew chain for ever. It is the exact
    // hazard `e7c10_g2` found and fixed for a *displaced* proxy focus, re-created
    // by every restart. Boot is the one moment where "no focus is live" is
    // provably true, so the narrowing is cleared here, before the derived indexes
    // are rebuilt, and the cleared documents are persisted.
    let mut repaired: Vec<ObjId> = Vec::new();
    for mut ha in raw_assignments {
        if !live(ha.host) {
            persist_del(ctx.out, crate::data::assignment_key(ha.host));
            continue;
        }
        let narrowed = ha.seats.values().any(|seat| seat.granted.is_some());
        if narrowed {
            for seat in ha.seats.values_mut() {
                seat.granted = None;
            }
            repaired.push(ha.host);
            persist_assignments(ctx.out, &ha);
        }
        let host = ha.host;
        let before = ha.seats.len();
        ha.seats.retain(|sid, seat| {
            live(seat.actor)
                && st
                    .stations
                    .get(&host)
                    .is_some_and(|hs| hs.stations.contains_key(sid))
        });
        if ha.seats.len() != before {
            persist_assignments(ctx.out, &ha);
        }
        if ha.seats.is_empty() {
            continue;
        }
        assigned_actors.extend(ha.seats.values().map(|seat| seat.actor));
        st.assignments.insert(ha.host, ha);
    }
    if !repaired.is_empty() {
        tracing::info!(
            target: "sim",
            units = %repaired
                .iter()
                .map(|id| format!("#{}", id.0))
                .collect::<Vec<_>>()
                .join(", "),
            "sim: cleared orphaned proxy-seat channel narrowings at boot (no focus survives a \
             restart, so no narrowing may)"
        );
    }
    st.derived = rebuild(&st);

    *ctx.w
        .module_state_mut::<SpatialState>()
        .expect("sim state slot installed at registration") = st;
    // D2: hydrate the declared-fallback RAM mirror the immediate controller and
    // `project_station_offer`'s relief branch read, bounded by the census. The
    // durable docs survive on disk regardless; this restores the live mirror so a
    // clean switch fires a fallback after a reboot without staff re-declaring.
    if !station_fallbacks.is_empty()
        && let Some(slot) = ctx.w.module_state_mut::<StationFallbacks>()
    {
        for rec in station_fallbacks {
            if !slot.declare(rec.unit, rec.station.clone(), rec.holder) {
                tracing::warn!(
                    "sim: station-fallback census reached at boot; extra declared fallbacks dropped"
                );
                break;
            }
        }
    }
    // E7c10 B8: hydrate the applied-marker book the `sim.*` fact provider reads,
    // bounded by `MAX_STATION_SERVICE_ROWS`. A marker that does not fit is
    // dropped from the mirror and stays on disk, and the fact then answers
    // `None` (unknown = unsatisfied) rather than a wrong `true`.
    if !station_services.is_empty()
        && let Some(book) = ctx.w.module_state_mut::<crate::seat::ServiceBook>()
    {
        for rec in station_services {
            if !book.record(rec) {
                tracing::warn!(
                    "sim: station-service census reached at boot; extra markers left on disk"
                );
                break;
            }
        }
    }
    // Republish control links from the installed assignments (M27 owner-republish).
    for actor in assigned_actors {
        crate::links::refresh_actor_links(ctx.w, actor);
    }
    Ok(quarantined)
}

/// Why a live placement is invalid, if it is.
fn validate_live(ctx: &ModuleCtx<'_>, st: &SpatialState, rec: &PlacementRecord) -> Option<String> {
    let w: &mush_core::World = ctx.w;
    let obj = rec.object;
    let gone = |id: ObjId| w.get(id).is_none_or(|o| o.flags.contains(Flags::GOING));
    if gone(obj) {
        return Some("object missing".into());
    }
    let participant = mush_core::BehaviorId::new("sim.participant").ok()?;
    if !w.has_behavior(obj, &participant) {
        return Some("missing sim.participant".into());
    }
    match &rec.parent {
        SpatialParent::Direct { domain, .. } => match st.domains.get(domain) {
            None => return Some(format!("domain #{domain} missing")),
            Some(d) if d.status == DomainStatus::Retired => {
                return Some(format!("domain #{domain} retired"));
            }
            Some(d) if gone(d.host) => return Some(format!("domain #{domain} host missing")),
            Some(d) if gone(d.presentation_location) => {
                return Some(format!("domain #{domain} presentation room missing"));
            }
            _ => {}
        },
        SpatialParent::Interior { space, slot } => {
            if gone(*space) {
                return Some(format!("space {space} missing"));
            }
            let Some(s) = st.spaces.get(space) else {
                return Some(format!("{space} has no space record"));
            };
            if let Some(slot) = slot
                && !s.compartments.contains_key(slot)
            {
                return Some(format!("{space} has no compartment {slot}"));
            }
        }
        SpatialParent::Attached { host, mount, .. } => {
            if gone(*host) {
                return Some(format!("host {host} missing"));
            }
            if st
                .mounts
                .get(host)
                .and_then(|m| m.mounts.get(mount))
                .is_none()
            {
                return Some(format!("{host} has no mount {mount}"));
            }
        }
        SpatialParent::Unplaced => {}
    }
    match ancestors(st, obj) {
        Err(e) => return Some(e.to_string()),
        Ok(up) => {
            if up.iter().any(|&a| gone(a)) {
                return Some("an ancestor is missing".into());
            }
        }
    }
    None
}
