//! Boot: rebuild [`SpatialState`] from the `sim` docs, validate every record
//! against the graph invariants, quarantine what fails, rebuild the indexes,
//! and reconcile projections (design §9.4, §13; cut-1 review).
//!
//! Policy: a corrupt `meta` or container doc (domain/space/mounts) FAILS the
//! boot — losing a container would strand everything under it (the comsys /
//! scenes precedent). A corrupt or dangling placement is quarantined to the
//! recovery room with a durable reason, never dropped.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Flags, ModError, ModuleBootData, ModuleCtx, ObjId, is_log_key};
use smol_str::SmolStr;

use crate::data::{
    DomainRecord, DomainStatus, HostMounts, InteriorSpaceRecord, META_KEY, PlacementRecord,
    SpaceAnchor, SpatialParent, SpatialState, UnplacedInfo, persist_del, persist_placement,
    persist_space, placement_key,
};
use crate::graph::{ancestors, project, rebuild};

fn parse_id(key: &str, prefix: &str) -> Option<u64> {
    key.strip_prefix(prefix)?.parse().ok()
}

/// Rebuild and install the state from boot data. Returns the number of
/// placements quarantined.
pub(crate) fn load_boot(ctx: &mut ModuleCtx<'_>, data: ModuleBootData, recovery_room: ObjId) -> Result<usize, ModError> {
    let mut st = SpatialState::new(recovery_room);
    let mut meta_next: Option<u64> = None;
    let mut placements: BTreeMap<ObjId, PlacementRecord> = BTreeMap::new();
    let mut unreadable: Vec<ObjId> = Vec::new();
    let mut log_docs: Vec<(SmolStr, serde_json::Value)> = Vec::new();

    for (key, value) in data.kv {
        if is_log_key(&key) {
            log_docs.push((key, value));
        } else if key == META_KEY {
            // A present-but-corrupt meta doc fails the boot: silently guessing
            // the allocator could reissue a live domain id.
            meta_next = Some(
                value
                    .get("next_domain_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| ModError::msg("sim: meta doc is corrupt (no next_domain_id)"))?,
            );
        } else if key.starts_with("domain:") {
            let d: DomainRecord = serde_json::from_value(value)
                .map_err(|e| ModError::msg(format!("sim: bad domain doc {key:?}: {e}")))?;
            if parse_id(&key, "domain:") != Some(d.id.0) {
                return Err(ModError::msg(format!("sim: doc {key:?} claims domain #{}", d.id)));
            }
            st.domains.insert(d.id, d);
        } else if key.starts_with("space:") {
            let s: InteriorSpaceRecord = serde_json::from_value(value)
                .map_err(|e| ModError::msg(format!("sim: bad space doc {key:?}: {e}")))?;
            st.spaces.insert(s.object, s);
        } else if key.starts_with("mounts:") {
            let m: HostMounts = serde_json::from_value(value)
                .map_err(|e| ModError::msg(format!("sim: bad mounts doc {key:?}: {e}")))?;
            st.mounts.insert(m.host, m);
        } else if let Some(rest) = key.strip_prefix("placement:") {
            match serde_json::from_value::<PlacementRecord>(value) {
                Ok(p) => {
                    placements.insert(p.object, p);
                }
                Err(e) => match rest.parse::<u32>() {
                    // Recover the object id from the key and quarantine it below.
                    Ok(n) => {
                        tracing::error!(%key, error = %e, "sim: unparseable placement doc; recovering by id");
                        unreadable.push(ObjId(n));
                    }
                    Err(_) => {
                        tracing::error!(%key, error = %e, "sim: unparseable placement doc with no readable id; deleting");
                        persist_del(ctx.out, key.clone());
                    }
                },
            }
        } else {
            tracing::warn!(%key, "sim: unknown doc ignored");
        }
    }
    st.recovery.restore_from_kv(&log_docs);
    st.next_domain_id = meta_next
        .unwrap_or(0)
        .max(st.domains.keys().last().map_or(0, |d| d.0 + 1));

    // Validate placements against the world and the graph.
    st.placements = placements;
    st.derived = rebuild(&st);
    let now = ctx.w.now();

    // A record whose object no longer exists is a tombstone: delete it rather
    // than write a zombie quarantine record nothing could ever recover.
    let mut delete: BTreeSet<ObjId> = unreadable.iter().copied().filter(|o| ctx.w.get(*o).is_none()).collect();
    // Failure roots: unreadable-but-live objects, plus records that fail live
    // validation. Each takes its whole subtree so no valid child is left under
    // an Unplaced parent.
    let mut roots: BTreeMap<ObjId, String> = BTreeMap::new();
    for o in &unreadable {
        if ctx.w.get(*o).is_some() {
            roots.insert(*o, "unparseable placement doc".to_string());
        }
    }
    let ids: Vec<ObjId> = st.placements.keys().copied().collect();
    for obj in ids {
        let Some(rec) = st.placements.get(&obj) else { continue };
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
                if n == *root { reason.clone() } else { format!("boot: ancestor {root} quarantined") }
            });
        }
    }
    for o in &delete {
        st.placements.remove(o);
        persist_del(ctx.out, placement_key(*o));
    }
    let quarantined = quarantine.len();
    for (obj, reason) in &quarantine {
        let prev = st.placements.get(obj);
        let q = PlacementRecord {
            object: *obj,
            parent: SpatialParent::Unplaced,
            revision: prev.map_or(1, |p| p.revision + 1),
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
        let Some(s) = st.spaces.get(&sid) else { continue };
        let dangling = match &s.anchor {
            SpaceAnchor::FixedDomain { domain, .. } => st
                .domains
                .get(domain)
                .is_none_or(|d| d.status == DomainStatus::Retired),
            SpaceAnchor::MovingObject { host } => ctx.w.get(*host).is_none_or(|o| o.flags.contains(Flags::GOING)),
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
        let Some(dest) = project(&st, ctx.w, obj) else { continue };
        if ctx.w.get(obj).and_then(|o| o.location) == Some(dest) {
            continue;
        }
        if ctx.w.get(obj).is_none() {
            continue;
        }
        match ctx.w.move_to(obj, dest) {
            Ok(()) => tracing::warn!(obj = %obj, dest = %dest, "sim: projection reconciled at boot"),
            Err(e) => tracing::error!(obj = %obj, dest = %dest, error = %e, "sim: could not reconcile projection"),
        }
    }

    *ctx.w
        .module_state_mut::<SpatialState>()
        .expect("sim state slot installed at registration") = st;
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
            Some(d) if d.status == DomainStatus::Retired => return Some(format!("domain #{domain} retired")),
            Some(d) if gone(d.host) => return Some(format!("domain #{domain} host missing")),
            Some(d) if gone(d.presentation_location) => return Some(format!("domain #{domain} presentation room missing")),
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
            if st.mounts.get(host).and_then(|m| m.mounts.get(mount)).is_none() {
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
