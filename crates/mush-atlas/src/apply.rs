//! The atlas's [`EntryApplier`]s (plan §3.1): how each place-shaped entry kind becomes
//! records and objects, is changed in place, and is retired without orphaning a
//! reference.
//!
//! One applier per kind, each with its **phase**, so `mush-content` orders the work:
//! a region before its localities, a neighborhood before its sites, sites before
//! structures, structures before the rooms on their floors, rooms before the exits
//! and items and features in them, and hooks and secrets last. Retire runs the
//! phases backwards.
//!
//! ## Refusals (the G lens)
//!
//! - two room footprints on one surface may not overlap (doc 14 P3); two site
//!   footprints in one neighborhood may not overlap (11a M9); a room on a floor must
//!   lie inside its structure;
//! - an exit between rooms whose footprints do not touch is a journey portal and
//!   needs a distance, else it is refused naming both rooms;
//! - a retire is refused while a room is occupied, holds something the pack did not
//!   put there, is the destination of an exit the pack did not make, or a module's
//!   `@destroy` veto refuses it (housing's titled property, a scene's anchor).
//!
//! ## Live state (doc 14 §3.1)
//!
//! Update touches only source-owned fields — names, descriptions, details, flags,
//! geometry — in place on the same object, so what happened in a room survives a
//! pack revision. Retire is the only path that destroys an object.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_content::{Appliers, EntryApplier, Pack, PackError, RawEntry};
use mush_core::{Attr, CanonicalId, Ctx, Flags, GOD, LIMBO, ObjId, ObjKind, WorldEvent};

use crate::def::{
    CheckpointDef, ConnectionDef, ExitDef, FeatureDef, HookDef, ItemDef, LocalityDef,
    NeighborhoodDef, PortalAt, RegionDef, Resolver, RoomDef, SecretDef, SiteDef, StructureDef,
    SurfaceDef,
};
use crate::geom::{Point3, PointCm, Shape, cm, metres};
use crate::module::{ATTR_NS, AtlasState, del_doc, put_doc};
use crate::raster::{self, Grid, SurfaceInput, overlap, shared_threshold, threshold_point};
use crate::record::{
    AccessPoint, Budget, CheckpointRec, ConnectionRec, Endpoint, FeatureRec, Frame, Geometry,
    HookRec, HydrationPolicy, KIND_CHECKPOINT, KIND_CONNECTION, KIND_FEATURE, KIND_HOOK,
    KIND_LOCALITY, KIND_NEIGHBORHOOD, KIND_PORTAL, KIND_REGION, KIND_SECRET, KIND_SITE,
    KIND_STRUCTURE, KIND_TILE, LegDef, LocalityRec, MAX_NAME_BYTES, MAX_TEXT_BYTES,
    NeighborhoodRec, PortalGeometry, PortalRec, RecordKey, RegionKind, RegionRec, ReviewState,
    RoomGeo, SecretRec, SiteOrigin, SiteRec, StructureRec, SurfaceRef, TileOwner, TileRec,
    TimePolicy, check_optional_text, check_text, record_key, roomgeo_key,
};

/// The entry kinds this crate owns, with their phases.
pub const KINDS: [(&str, u8); 13] = [
    ("region", 0),
    ("locality", 1),
    ("checkpoint", 2),
    ("connection", 3),
    ("neighborhood", 4),
    ("site", 5),
    ("structure", 6),
    ("room", 7),
    ("feature", 8),
    ("item", 9),
    ("exit", 10),
    ("hook", 11),
    ("secret", 11),
];

/// The most details a room or item may carry.
pub const MAX_DETAILS: usize = 32;

/// The most bytes in one detail.
pub const MAX_DETAIL_BYTES: usize = 2 * 1024;

/// The staff lock word on an exit.
pub const STAFF_LOCK: &str = "flag^WIZARD|flag^ROYALTY";

/// Register every atlas applier under this crate's name.
///
/// # Errors
///
/// [`PackError::KindOwnedElsewhere`] when another owner already declared a kind.
pub fn register_appliers(appliers: &mut Appliers) -> Result<(), PackError> {
    for (kind, phase) in KINDS {
        let kind = CanonicalId::new(kind).map_err(|e| PackError::EntryDecode {
            kind: kind.to_string(),
            detail: e.to_string(),
        })?;
        appliers.register(crate::module::NS, Arc::new(KindApplier { kind, phase }))?;
    }
    Ok(())
}

/// One kind's applier.
struct KindApplier {
    kind: CanonicalId,
    phase: u8,
}

/// Run `f` with the atlas state taken out of its slot and put back afterwards.
fn with_atlas<T>(
    ctx: &mut Ctx<'_>,
    f: impl FnOnce(&mut Ctx<'_>, &mut AtlasState) -> Result<T, String>,
) -> Result<T, String> {
    let Some(slot) = ctx.state_mut::<AtlasState>() else {
        return Err("the atlas is not composed on this world".to_string());
    };
    let mut state = std::mem::take(slot);
    let result = f(ctx, &mut state);
    if let Some(slot) = ctx.state_mut::<AtlasState>() {
        *slot = state;
    }
    result
}

impl EntryApplier for KindApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        vec![self.kind.clone()]
    }

    fn phase(&self) -> u8 {
        self.phase
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let kind = self.kind.as_str().to_string();
        with_atlas(ctx, |ctx, state| {
            create_entry(&kind, ctx, state, pack, id, entry, placed, None)
        })
    }

    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let kind = self.kind.as_str().to_string();
        with_atlas(ctx, |ctx, state| match kind.as_str() {
            // In place: the object keeps its live state.
            "room" | "item" => create_entry(&kind, ctx, state, pack, id, entry, placed, existing),
            // Records: replaced under the same key, revision up.
            "region" | "locality" | "checkpoint" | "connection" | "neighborhood" | "site"
            | "structure" | "feature" | "hook" | "secret" => {
                create_entry(&kind, ctx, state, pack, id, entry, placed, existing)
            }
            // Exits: remade (their live state is nothing).
            _ => {
                let key = RecordKey::new(pack.manifest().id().clone(), id.clone());
                retire_check_entry(&kind, ctx, state, &key, existing, placed)?;
                retire_entry(&kind, ctx, state, &key, existing)?;
                create_entry(&kind, ctx, state, pack, id, entry, placed, None)
            }
        })
    }

    fn retire_check(
        &self,
        ctx: &Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        let Some(state) = ctx.state::<AtlasState>() else {
            return Err("the atlas is not composed on this world".to_string());
        };
        let key = RecordKey::new(pack.clone(), id.clone());
        retire_check_entry(self.kind.as_str(), ctx, state, &key, existing, placed)
    }

    fn retire(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
    ) -> Result<(), String> {
        let kind = self.kind.as_str().to_string();
        let key = RecordKey::new(pack.clone(), id.clone());
        with_atlas(ctx, |ctx, state| {
            retire_entry(&kind, ctx, state, &key, existing)
        })
    }

    fn finish(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        match self.kind.as_str() {
            "neighborhood" => with_atlas(ctx, |ctx, state| finish_neighborhood(ctx, state, pack)),
            "region" => with_atlas(ctx, |ctx, state| finish_region(ctx, state, pack)),
            "site" => with_atlas(ctx, |ctx, state| finish_sites(ctx, state, pack)),
            _ => Ok(()),
        }
    }

    fn finish_retire(&self, ctx: &mut Ctx<'_>, pack: &CanonicalId) -> Result<(), String> {
        if self.kind.as_str() != "region" {
            return Ok(());
        }
        with_atlas(ctx, |ctx, state| {
            // Whatever the entries did not take with them: derived tiles and walls.
            for key in state.records.keys_of_pack(pack) {
                del_doc(ctx, key);
            }
            drop_pack_records(state, pack);
            Ok(())
        })
    }
}

/// Every record of `pack`, gone from RAM.
fn drop_pack_records(state: &mut AtlasState, pack: &CanonicalId) {
    let r = &mut state.records;
    r.regions.retain(|k, _| &k.pack != pack);
    r.localities.retain(|k, _| &k.pack != pack);
    r.neighborhoods.retain(|k, _| &k.pack != pack);
    r.sites.retain(|k, _| &k.pack != pack);
    r.structures.retain(|k, _| &k.pack != pack);
    r.checkpoints.retain(|k, _| &k.pack != pack);
    r.connections.retain(|k, _| &k.pack != pack);
    r.features.retain(|k, _| &k.pack != pack);
    r.portals.retain(|k, _| &k.pack != pack);
    r.tiles.retain(|k, _| &k.pack != pack);
    r.hooks.retain(|k, _| &k.pack != pack);
    r.secrets.retain(|k, _| &k.pack != pack);
    r.roomgeo.retain(|_, g| &g.key.pack != pack);
}

// ---------------------------------------------------------------------------
// Create / update
// ---------------------------------------------------------------------------

fn decode<T: serde::de::DeserializeOwned>(id: &CanonicalId, entry: &RawEntry) -> Result<T, String> {
    entry.decode::<T>().map_err(|e| format!("{id}: {e}"))
}

/// The pack's own neighborhood key, which every site, room, feature, hook and secret
/// of a neighborhood pack belongs to.
fn own_neighborhood(state: &AtlasState, pack: &Pack) -> Result<RecordKey, String> {
    let kind = CanonicalId::new("neighborhood").map_err(|e| e.to_string())?;
    let pack_id = pack.manifest().id();
    let mut found = pack.entries_of(&kind).map(|(id, _)| id.clone());
    let entry = found
        .next()
        .ok_or_else(|| format!("{pack_id} has no neighborhood entry"))?;
    if found.next().is_some() {
        return Err(format!("{pack_id} has more than one neighborhood entry"));
    }
    let key = RecordKey::new(pack_id.clone(), entry);
    if state.records.neighborhoods.contains_key(&key) {
        Ok(key)
    } else {
        Err(format!("{pack_id}'s neighborhood is not applied yet"))
    }
}

fn revision_after(previous: Option<u64>) -> u64 {
    previous.map_or(1, |r| r.saturating_add(1))
}

#[allow(clippy::too_many_arguments)]
fn create_entry(
    kind: &str,
    ctx: &mut Ctx<'_>,
    state: &mut AtlasState,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
    existing: Option<ObjId>,
) -> Result<Option<ObjId>, String> {
    let key = RecordKey::new(pack.manifest().id().clone(), id.clone());
    match kind {
        "region" => {
            let def: RegionDef = decode(id, entry)?;
            let previous = state.records.regions.get(&key).map(|r| r.revision);
            // A staff-set theme survives a re-apply that authors none.
            let theme = def.theme.clone().or_else(|| {
                state
                    .records
                    .regions
                    .get(&key)
                    .and_then(|r| r.theme.clone())
            });
            let extent = def
                .extent
                .validate()
                .map_err(|e| format!("{id}: extent: {e}"))?;
            let datum_cm = i32::try_from(cm(def.datum_m).map_err(|e| format!("{id}: datum: {e}"))?)
                .map_err(|_| format!("{id}: datum is out of range"))?;
            let kind = def.region_kind().map_err(|e| format!("{id}: {e}"))?;
            let frame = matches!(kind, RegionKind::Surface { .. }).then(Frame::default);
            let rec = RegionRec {
                key: key.clone(),
                name: def.name,
                kind,
                frame,
                parent: None,
                extent,
                datum_cm,
                background: def.background,
                ground: Vec::new(),
                ground_resolution_cm: def.ground_resolution_cm,
                theme,
                revision: revision_after(previous),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_REGION, &key), &rec);
            state.records.regions.insert(key, rec);
            Ok(None)
        }
        "locality" => {
            let def: LocalityDef = decode(id, entry)?;
            let region = Resolver::new(pack, placed, &state.records)
                .record(&def.region, "region")
                .map_err(|e| format!("{id}: region: {e}"))?;
            let previous = state.records.localities.get(&key).map(|r| r.revision);
            let theme = def.theme.clone().or_else(|| {
                state
                    .records
                    .localities
                    .get(&key)
                    .and_then(|r| r.theme.clone())
            });
            let rec = LocalityRec {
                key: key.clone(),
                region,
                name: def.name,
                kind: def.kind,
                footprint: def.footprint,
                theme,
                revision: revision_after(previous),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_LOCALITY, &key), &rec);
            state.records.localities.insert(key, rec);
            Ok(None)
        }
        "checkpoint" => {
            let def: CheckpointDef = decode(id, entry)?;
            let region = Resolver::new(pack, placed, &state.records)
                .record(&def.region, "region")
                .map_err(|e| format!("{id}: region: {e}"))?;
            let previous = state.records.checkpoints.get(&key).map(|r| r.revision);
            let rec = CheckpointRec {
                key: key.clone(),
                region,
                name: def.name,
                point: def.point,
                revision: revision_after(previous),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_CHECKPOINT, &key), &rec);
            state.records.checkpoints.insert(key, rec);
            Ok(None)
        }
        "connection" => {
            let def: ConnectionDef = decode(id, entry)?;
            let resolver = Resolver::new(pack, placed, &state.records);
            let region = resolver
                .record(&def.region, "region")
                .map_err(|e| format!("{id}: region: {e}"))?;
            let mut legs = Vec::with_capacity(def.legs.len());
            let last = def.legs.len().saturating_sub(1);
            for (i, leg) in def.legs.iter().enumerate() {
                // The last leg names the connection itself: its far end (S2).
                let to = if i == last && leg.to == id.as_str() {
                    key.clone()
                } else {
                    resolver
                        .record(&leg.to, "checkpoint")
                        .map_err(|e| format!("{id}: leg: {e}"))?
                };
                legs.push(LegDef {
                    to,
                    base_s: leg.base_s,
                    hazard: leg.hazard,
                    interdiction: leg.interdiction,
                });
            }
            let entry_ref = |r: &crate::def::EntryRefDef| -> Result<RecordKey, String> {
                Ok(RecordKey::new(
                    CanonicalId::new(&r.pack).map_err(|e| format!("{id}: service pack: {e}"))?,
                    CanonicalId::new(&r.entry).map_err(|e| format!("{id}: service entry: {e}"))?,
                ))
            };
            let service = match &def.service {
                Some(s) => Some(crate::record::ServiceRec {
                    vehicle: entry_ref(&s.vehicle)?,
                    dwell_s: s.dwell_s,
                    driver: s.driver.as_ref().map(entry_ref).transpose()?,
                }),
                None => None,
            };
            let access = |a: &crate::def::AccessDef| -> Result<AccessPoint, String> {
                Ok(AccessPoint {
                    neighborhood: CanonicalId::new(&a.neighborhood)
                        .map_err(|e| format!("{id}: endpoint neighborhood: {e}"))?,
                    room: None,
                    point: a.point,
                })
            };
            let endpoints = (access(&def.from)?, access(&def.to)?);
            let previous = state.records.connections.get(&key).map(|r| r.revision);
            let rec = ConnectionRec {
                key: key.clone(),
                region,
                name: def.name,
                kind: def.kind,
                endpoints,
                direction: def.direction,
                modes: def.modes,
                corridor: def.corridor,
                legs,
                cost: def.cost,
                visibility: def.visibility,
                status: def.status,
                service,
                revision: revision_after(previous),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_CONNECTION, &key), &rec);
            state.records.connections.insert(key, rec);
            Ok(None)
        }
        "neighborhood" => {
            let def: NeighborhoodDef = decode(id, entry)?;
            let kind_id = CanonicalId::new("neighborhood").map_err(|e| e.to_string())?;
            if pack.entries_of(&kind_id).count() != 1 {
                return Err(format!(
                    "{}: a neighborhood pack has exactly one neighborhood entry",
                    pack.manifest().id()
                ));
            }
            let resolver = Resolver::new(pack, placed, &state.records);
            let region = resolver
                .record(&def.region, "region")
                .map_err(|e| format!("{id}: region: {e}"))?;
            let locality = resolver
                .record(&def.locality, "locality")
                .map_err(|e| format!("{id}: locality: {e}"))?;
            let previous = state.records.neighborhoods.get(&key).cloned();
            let rec = NeighborhoodRec {
                key: key.clone(),
                region,
                locality,
                name: def.name,
                setting: def.setting,
                representation: def.representation,
                footprint: def.footprint,
                room_budget: Budget {
                    target: def.room_budget.target,
                    maximum: def.room_budget.maximum,
                },
                hydration: HydrationPolicy {
                    enabled: def.hydration.enabled,
                    max_player_sites: def.hydration.max_player_sites,
                },
                zone: previous.as_ref().and_then(|p| p.zone),
                reviewers: previous
                    .as_ref()
                    .map(|p| p.reviewers.clone())
                    .unwrap_or_default(),
                tiles: previous
                    .as_ref()
                    .map(|p| p.tiles.clone())
                    .unwrap_or_default(),
                resolution_cm: def.resolution_cm,
                theme: def
                    .theme
                    .clone()
                    .or_else(|| previous.as_ref().and_then(|p| p.theme.clone())),
                revision: revision_after(previous.map(|p| p.revision)),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_NEIGHBORHOOD, &key), &rec);
            state.records.neighborhoods.insert(key, rec);
            Ok(None)
        }
        "site" => {
            let def: SiteDef = decode(id, entry)?;
            let neighborhood = own_neighborhood(state, pack).map_err(|e| format!("{id}: {e}"))?;
            let resolution = state
                .records
                .neighborhoods
                .get(&neighborhood)
                .map_or(100, |n| n.resolution_cm);
            let shape = Shape::Polygon(
                def.footprint
                    .to_cm()
                    .map_err(|e| format!("{id}: footprint: {e}"))?,
            );
            for other in state.records.sites_in(&neighborhood) {
                if other.key == key {
                    continue;
                }
                let Ok(other_shape) = other.footprint.to_cm() else {
                    continue;
                };
                if overlap(&shape, &Shape::Polygon(other_shape), resolution) {
                    return Err(format!(
                        "{id}: its footprint overlaps site {} ({})",
                        other.key, other.name
                    ));
                }
            }
            let exit_kind = CanonicalId::new("exit").map_err(|e| e.to_string())?;
            let mut entrances = Vec::with_capacity(def.entrances.len());
            for reference in &def.entrances {
                let entry_id = CanonicalId::new(reference)
                    .map_err(|e| format!("{id}: entrance {reference:?}: {e}"))?;
                if !pack
                    .entry(&entry_id)
                    .is_some_and(|e| e.kind() == &exit_kind)
                {
                    return Err(format!(
                        "{id}: entrance {reference:?} is not an exit entry of this pack"
                    ));
                }
                entrances.push(RecordKey::new(pack.manifest().id().clone(), entry_id));
            }
            let previous = state.records.sites.get(&key).cloned();
            let rec = SiteRec {
                key: key.clone(),
                neighborhoods: vec![neighborhood],
                name: def.name,
                kind: def.site_kind,
                footprint: def.footprint,
                entrances,
                property: previous.as_ref().and_then(|p| p.property),
                origin: SiteOrigin::Authored,
                review: ReviewState::NotApplicable,
                tactical_ready: false,
                theme: def
                    .theme
                    .clone()
                    .or_else(|| previous.as_ref().and_then(|p| p.theme.clone())),
                revision: revision_after(previous.map(|p| p.revision)),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_SITE, &key), &rec);
            state.records.sites.insert(key, rec);
            Ok(None)
        }
        "structure" => {
            let def: StructureDef = decode(id, entry)?;
            let site = Resolver::new(pack, placed, &state.records)
                .record(&def.site, "site")
                .map_err(|e| format!("{id}: site: {e}"))?;
            let region = region_of_site(state, &site)
                .ok_or_else(|| format!("{id}: site {site} has no region"))?;
            let previous = state.records.structures.get(&key).cloned();
            let mut floors = Vec::with_capacity(def.floors.len());
            for f in &def.floors {
                floors.push(crate::record::Floor {
                    index: f.index,
                    z_offset_m: f.z_offset_m,
                    height_m: f.height_m,
                    name: f.name.clone(),
                    tile: None,
                });
            }
            let rec = StructureRec {
                key: key.clone(),
                site,
                footprint: def.footprint,
                floors,
                revision: revision_after(previous.map(|p| p.revision)),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            // The walls, derived: one closed ring per floor.
            for wall in raster::wall_features(&rec, &region) {
                put_doc(ctx, record_key(KIND_FEATURE, &wall.key), &wall);
                state.records.features.insert(wall.key.clone(), wall);
            }
            put_doc(ctx, record_key(KIND_STRUCTURE, &key), &rec);
            state.records.structures.insert(key, rec);
            Ok(None)
        }
        "room" => create_room(ctx, state, pack, id, entry, placed, existing, key).map(Some),
        "feature" => {
            let def: FeatureDef = decode(id, entry)?;
            let neighborhood = own_neighborhood(state, pack).map_err(|e| format!("{id}: {e}"))?;
            let region = state
                .records
                .neighborhoods
                .get(&neighborhood)
                .map(|n| n.region.clone())
                .ok_or_else(|| format!("{id}: no region"))?;
            let resolver = Resolver::new(pack, placed, &state.records);
            let surface = surface_of(&resolver, &state.records, &def.surface)
                .map_err(|e| format!("{id}: {e}"))?;
            let room = match &def.room {
                Some(r) => Some(resolver.room(r).map_err(|e| format!("{id}: room: {e}"))?),
                None => None,
            };
            let geometry = match (def.polyline, def.polygon, def.point) {
                (Some(points), None, None) => Geometry::Polyline(points),
                (None, Some(poly), None) => Geometry::Polygon(poly),
                (None, None, Some(p)) => Geometry::Point(p),
                _ => {
                    return Err(format!(
                        "{id}: a feature has exactly one of polyline, polygon, point"
                    ));
                }
            };
            let previous = state.records.features.get(&key).map(|r| r.revision);
            let rec = FeatureRec {
                key: key.clone(),
                region,
                surface,
                kind: def.feature_kind,
                geometry,
                z_range: def.z_range,
                blocks: def.blocks,
                state: crate::record::FeatureState::Intact,
                raise_cm: def.raise_cm,
                room,
                revision: revision_after(previous),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_FEATURE, &key), &rec);
            state.records.features.insert(key, rec);
            Ok(None)
        }
        "item" => create_item(ctx, state, pack, id, entry, placed, existing).map(Some),
        "exit" => create_exit(ctx, state, pack, id, entry, placed, key).map(Some),
        "hook" => {
            let def: HookDef = decode(id, entry)?;
            let room = match &def.room {
                Some(r) => Some(
                    Resolver::new(pack, placed, &state.records)
                        .room(r)
                        .map_err(|e| format!("{id}: room: {e}"))?,
                ),
                None => None,
            };
            let previous = state.records.hooks.get(&key).map(|r| r.revision);
            let rec = HookRec {
                key: key.clone(),
                name: def.name,
                text: def.text,
                room,
                revision: revision_after(previous),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_HOOK, &key), &rec);
            state.records.hooks.insert(key, rec);
            Ok(None)
        }
        "secret" => {
            let def: SecretDef = decode(id, entry)?;
            let mut about = Vec::with_capacity(def.about.len());
            for a in &def.about {
                about.push(CanonicalId::new(a).map_err(|e| format!("{id}: about {a:?}: {e}"))?);
            }
            let previous = state.records.secrets.get(&key).map(|r| r.revision);
            let rec = SecretRec {
                key: key.clone(),
                truth: def.truth,
                reveal: def.reveal,
                about,
                revision: revision_after(previous),
            };
            rec.validate().map_err(|e| format!("{id}: {e}"))?;
            put_doc(ctx, record_key(KIND_SECRET, &key), &rec);
            state.records.secrets.insert(key, rec);
            Ok(None)
        }
        other => Err(format!("{id}: the atlas does not own kind {other}")),
    }
}

fn region_of_site(state: &AtlasState, site: &RecordKey) -> Option<RecordKey> {
    let s = state.records.sites.get(site)?;
    let nb = s.neighborhoods.first()?;
    state
        .records
        .neighborhoods
        .get(nb)
        .map(|n| n.region.clone())
}

fn surface_of(
    resolver: &Resolver<'_>,
    records: &crate::record::Records,
    def: &SurfaceDef,
) -> Result<SurfaceRef, String> {
    match def {
        SurfaceDef::Ground => Ok(SurfaceRef::Ground),
        SurfaceDef::Floor(structure, index) => {
            let key = resolver
                .record(structure, "structure")
                .map_err(|e| format!("surface: {e}"))?;
            let s = records
                .structures
                .get(&key)
                .ok_or_else(|| format!("surface: {structure} is not applied"))?;
            if s.floor(*index).is_none() {
                return Err(format!("surface: {structure} has no floor {index}"));
            }
            Ok(SurfaceRef::Floor {
                structure: key,
                index: *index,
            })
        }
    }
}

fn parse_flags(words: &[String], allowed: &[(&str, Flags)]) -> Result<Flags, String> {
    let mut flags = Flags::empty();
    for word in words {
        let w = word.trim().to_ascii_lowercase();
        let Some((_, flag)) = allowed.iter().find(|(name, _)| *name == w) else {
            return Err(format!("flag {word:?} is not one a pack may set here"));
        };
        flags |= *flag;
    }
    Ok(flags)
}

const ROOM_FLAGS: [(&str, Flags); 5] = [
    ("outdoors", Flags::OUTDOORS),
    ("sheltered", Flags::SHELTERED),
    ("dark", Flags::DARK),
    ("safe", Flags::SAFE),
    ("jump_ok", Flags::JUMP_OK),
];

const ITEM_FLAGS: [(&str, Flags); 4] = [
    ("enter_ok", Flags::ENTER_OK),
    ("opaque", Flags::OPAQUE),
    ("sticky", Flags::STICKY),
    ("safe", Flags::SAFE),
];

fn check_details(id: &CanonicalId, details: &BTreeMap<String, String>) -> Result<(), String> {
    if details.len() > MAX_DETAILS {
        return Err(format!("{id}: more than {MAX_DETAILS} details"));
    }
    for (name, text) in details {
        if name.trim().is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b' ')
        {
            return Err(format!(
                "{id}: detail name {name:?} is not a short plain name"
            ));
        }
        if text.trim().is_empty() || text.len() > MAX_DETAIL_BYTES {
            return Err(format!(
                "{id}: detail {name:?} is empty or over {MAX_DETAIL_BYTES} bytes"
            ));
        }
    }
    Ok(())
}

/// Write the source-owned fields of an object: name, desc, details, flags.
fn dress(
    ctx: &mut Ctx<'_>,
    obj: ObjId,
    name: &str,
    desc: &str,
    details: &BTreeMap<String, String>,
    flags: Flags,
    all_flags: Flags,
) -> Result<(), String> {
    if ctx.w.get(obj).is_some_and(|o| o.name != name) {
        ctx.w.set_name(obj, name).map_err(|e| e.to_string())?;
    }
    if desc.trim().is_empty() {
        ctx.w.remove_attr(obj, "desc").map_err(|e| e.to_string())?;
    } else {
        ctx.w
            .set_attr(obj, "desc", Attr::text(desc, GOD))
            .map_err(|e| e.to_string())?;
    }
    // Details: exactly the pack's, no stale ones.
    let stale: Vec<String> = ctx
        .w
        .get(obj)
        .map(|o| {
            o.attrs
                .iter_namespace(ATTR_NS)
                .map(|(k, _)| k.as_str().to_string())
                .collect()
        })
        .unwrap_or_default();
    for attr in stale {
        ctx.w.remove_attr(obj, &attr).map_err(|e| e.to_string())?;
    }
    for (detail, text) in details {
        let attr = format!(
            "{ATTR_NS}:detail:{}",
            detail.trim().to_ascii_lowercase().replace(' ', "_")
        );
        ctx.w
            .set_attr(obj, &attr, Attr::text(text.as_str(), GOD))
            .map_err(|e| e.to_string())?;
    }
    for (_, flag) in all_flags.iter_names() {
        ctx.w
            .set_flag(obj, flag, flags.contains(flag))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_room(
    ctx: &mut Ctx<'_>,
    state: &mut AtlasState,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
    existing: Option<ObjId>,
    key: RecordKey,
) -> Result<ObjId, String> {
    let def: RoomDef = decode(id, entry)?;
    check_text("room.name", &def.name, MAX_NAME_BYTES).map_err(|e| format!("{id}: {e}"))?;
    check_optional_text("room.desc", &def.desc, MAX_TEXT_BYTES)
        .map_err(|e| format!("{id}: {e}"))?;
    check_details(id, &def.details)?;
    let kind = def.room_kind();
    let mut flags = parse_flags(&def.flags, &ROOM_FLAGS).map_err(|e| format!("{id}: {e}"))?;
    if kind == crate::record::RoomKind::Outdoor && !flags.contains(Flags::SHELTERED) {
        flags |= Flags::OUTDOORS;
    }
    let neighborhood = own_neighborhood(state, pack).map_err(|e| format!("{id}: {e}"))?;
    let nb = state
        .records
        .neighborhoods
        .get(&neighborhood)
        .cloned()
        .ok_or_else(|| format!("{id}: no neighborhood"))?;
    let resolver = Resolver::new(pack, placed, &state.records);
    let surface =
        surface_of(&resolver, &state.records, &def.surface).map_err(|e| format!("{id}: {e}"))?;
    let site = match &def.site {
        Some(s) => Some(
            resolver
                .record(s, "site")
                .map_err(|e| format!("{id}: site: {e}"))?,
        ),
        None => None,
    };
    let footprint = def.footprint.to_footprint();
    let shape = footprint
        .shape()
        .map_err(|e| format!("{id}: footprint: {e}"))?;
    // A room on a floor lies inside its structure.
    if let SurfaceRef::Floor { structure, .. } = &surface
        && let Some(s) = state.records.structures.get(structure)
        && let Ok(slab) = s.footprint.to_cm()
    {
        let grid = Grid::fitting(shape.bbox(), nb.resolution_cm, state.limits.tile_max_cells)
            .ok_or_else(|| format!("{id}: footprint is too large to rasterize"))?;
        if grid
            .cells_in(&shape)
            .iter()
            .any(|(x, y)| !slab.contains(grid.centre(*x, *y)))
        {
            return Err(format!("{id}: its footprint leaves structure {structure}"));
        }
    }
    // Rooms partition their surface (P3), across every pack in the region.
    for other in state.records.roomgeo.values() {
        if Some(other.room) == existing || other.surface != surface {
            continue;
        }
        let same_region = state
            .records
            .neighborhoods
            .get(&other.neighborhood)
            .is_some_and(|n| n.region == nb.region);
        if !same_region {
            continue;
        }
        let Ok(other_shape) = other.footprint.shape() else {
            continue;
        };
        if overlap(&shape, &other_shape, nb.resolution_cm) {
            return Err(format!(
                "{id}: its footprint overlaps room {} ({})",
                other.key,
                mush_core::output::display_name(ctx.w, other.room)
            ));
        }
    }
    let all_flags = ROOM_FLAGS
        .iter()
        .fold(Flags::empty(), |acc, (_, f)| acc | *f);
    let room = match existing.filter(|o| ctx.w.get(*o).is_some_and(|o| o.kind == ObjKind::Room)) {
        Some(room) => room,
        None => {
            let room = ctx
                .w
                .create(ObjKind::Room, &def.name, GOD, None)
                .map_err(|e| format!("{id}: {e}"))?;
            ctx.emit_world_event(&WorldEvent::ObjCreated {
                obj: room,
                kind: ObjKind::Room,
                creator: GOD,
            });
            room
        }
    };
    dress(
        ctx,
        room,
        &def.name,
        &def.desc,
        &def.details,
        flags,
        all_flags,
    )
    .map_err(|e| format!("{id}: {e}"))?;
    let previous = state.records.roomgeo.get(&room).map(|g| g.revision);
    let geo = RoomGeo {
        room,
        key: key.clone(),
        neighborhood,
        site,
        surface,
        footprint,
        kind,
        access: def.access,
        revision: revision_after(previous),
    };
    geo.validate().map_err(|e| format!("{id}: {e}"))?;
    put_doc(ctx, roomgeo_key(room), &geo);
    state.records.roomgeo.insert(room, geo);
    Ok(room)
}

fn create_item(
    ctx: &mut Ctx<'_>,
    state: &mut AtlasState,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
    existing: Option<ObjId>,
) -> Result<ObjId, String> {
    let def: ItemDef = decode(id, entry)?;
    check_text("item.name", &def.name, MAX_NAME_BYTES).map_err(|e| format!("{id}: {e}"))?;
    check_optional_text("item.desc", &def.desc, MAX_TEXT_BYTES)
        .map_err(|e| format!("{id}: {e}"))?;
    check_details(id, &def.details)?;
    let flags = parse_flags(&def.flags, &ITEM_FLAGS).map_err(|e| format!("{id}: {e}"))?;
    let room = Resolver::new(pack, placed, &state.records)
        .room(&def.room)
        .map_err(|e| format!("{id}: room: {e}"))?;
    let all_flags = ITEM_FLAGS
        .iter()
        .fold(Flags::empty(), |acc, (_, f)| acc | *f);
    let thing = match existing.filter(|o| ctx.w.get(*o).is_some_and(|o| o.kind == ObjKind::Thing)) {
        Some(thing) => {
            // Moved by a player: the pack does not fetch it back (live state).
            thing
        }
        None => {
            let thing = ctx
                .w
                .create(ObjKind::Thing, &def.name, GOD, Some(room))
                .map_err(|e| format!("{id}: {e}"))?;
            ctx.emit_world_event(&WorldEvent::ObjCreated {
                obj: thing,
                kind: ObjKind::Thing,
                creator: GOD,
            });
            thing
        }
    };
    dress(
        ctx,
        thing,
        &def.name,
        &def.desc,
        &def.details,
        flags,
        all_flags,
    )
    .map_err(|e| format!("{id}: {e}"))?;
    Ok(thing)
}

/// Create one exit object `spec` (`"Name <A>;a;alias"`) from `src` to `dest`.
fn open_exit(ctx: &mut Ctx<'_>, src: ObjId, spec: &str, dest: ObjId) -> Result<ObjId, String> {
    let mut segs = spec.split(';').map(str::trim).filter(|s| !s.is_empty());
    let name = segs
        .next()
        .ok_or_else(|| "an exit needs a name".to_string())?;
    if name.len() > MAX_NAME_BYTES {
        return Err(format!("exit name {name:?} is over {MAX_NAME_BYTES} bytes"));
    }
    let aliases: Vec<String> = segs.map(str::to_string).collect();
    let exit = ctx
        .w
        .create(ObjKind::Exit, name, GOD, Some(src))
        .map_err(|e| e.to_string())?;
    for a in aliases {
        ctx.w.add_alias(exit, &a).map_err(|e| e.to_string())?;
    }
    ctx.w
        .link_exit(exit, Some(dest))
        .map_err(|e| e.to_string())?;
    ctx.emit_world_event(&WorldEvent::ObjCreated {
        obj: exit,
        kind: ObjKind::Exit,
        creator: GOD,
    });
    Ok(exit)
}

fn point3(p: PointCm, z: f64) -> Point3 {
    Point3(metres(p.x), metres(p.y), z)
}

#[allow(clippy::too_many_lines)]
fn create_exit(
    ctx: &mut Ctx<'_>,
    state: &mut AtlasState,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
    key: RecordKey,
) -> Result<ObjId, String> {
    let def: ExitDef = decode(id, entry)?;
    if def.names.is_empty() || def.names.len() > 2 {
        return Err(format!(
            "{id}: an exit row names one exit, or two (there and back)"
        ));
    }
    let resolver = Resolver::new(pack, placed, &state.records);
    let from = resolver
        .room(&def.from)
        .map_err(|e| format!("{id}: from: {e}"))?;
    let to = resolver
        .room(&def.to)
        .map_err(|e| format!("{id}: to: {e}"))?;
    if from == to {
        return Err(format!("{id}: an exit joins two different rooms"));
    }
    let lock = match def.lock.as_deref().map(str::trim) {
        None | Some("") => None,
        Some("staff") => Some(
            mush_core::lock::parse_lock(STAFF_LOCK, |_| None)
                .map_err(|e| format!("{id}: lock: {e}"))?,
        ),
        Some(expr) => Some(
            mush_core::lock::parse_lock(expr, |_| None).map_err(|e| format!("{id}: lock: {e}"))?,
        ),
    };
    if let Some(desc) = &def.desc {
        check_optional_text("exit.desc", desc, MAX_TEXT_BYTES).map_err(|e| format!("{id}: {e}"))?;
    }
    // Geometry, before any object is made.
    let geo_from = state.records.roomgeo.get(&from).cloned();
    let geo_to = state.records.roomgeo.get(&to).cloned();
    let (geometry, time, distance_m) = match &def.portal.at {
        Some(PortalAt::Point(p)) => (
            PortalGeometry::Point(*p),
            TimePolicy::Instant,
            def.portal.distance_m.unwrap_or(0),
        ),
        Some(PortalAt::Segment(a, b)) => (
            PortalGeometry::Segment(*a, *b),
            TimePolicy::Instant,
            def.portal.distance_m.unwrap_or(0),
        ),
        None => match (&geo_from, &geo_to) {
            (Some(a), Some(b)) if a.surface == b.surface => {
                let sa = a.footprint.shape().map_err(|e| format!("{id}: {e}"))?;
                let sb = b.footprint.shape().map_err(|e| format!("{id}: {e}"))?;
                match shared_threshold(&sa, &sb) {
                    Some((s, e)) => (
                        PortalGeometry::Segment(point3(s, 0.0), point3(e, 0.0)),
                        TimePolicy::Instant,
                        0,
                    ),
                    None => match def.portal.distance_m {
                        Some(d) => (
                            PortalGeometry::Segment(
                                point3(sa.centre(), 0.0),
                                point3(sb.centre(), 0.0),
                            ),
                            TimePolicy::Timed {
                                modes: vec!["foot".to_string()],
                            },
                            d,
                        ),
                        None => {
                            return Err(format!(
                                "{id}: the footprints of {} and {} do not touch; give portal.at or portal.distance_m",
                                a.key, b.key
                            ));
                        }
                    },
                }
            }
            (Some(a), Some(b)) => {
                return Err(format!(
                    "{id}: {} and {} are on different surfaces; give portal.at",
                    a.key, b.key
                ));
            }
            // Limbo, which has no geometry: the portal sits at the other room.
            (Some(a), None) | (None, Some(a)) => {
                let s = a.footprint.shape().map_err(|e| format!("{id}: {e}"))?;
                (
                    PortalGeometry::Point(point3(s.centre(), 0.0)),
                    TimePolicy::Instant,
                    0,
                )
            }
            (None, None) => {
                return Err(format!(
                    "{id}: neither {} nor {} has geometry",
                    def.from, def.to
                ));
            }
        },
    }; // S2: a timed portal is a journey, whatever its geometry said.
    let time = if def.portal.timed.is_empty() {
        time
    } else {
        TimePolicy::Timed {
            modes: def.portal.timed.clone(),
        }
    };

    if from != LIMBO && geo_from.is_none() {
        return Err(format!("{id}: {} is not an atlas room", def.from));
    }
    if to != LIMBO && geo_to.is_none() {
        return Err(format!("{id}: {} is not an atlas room", def.to));
    }
    // The objects.
    let mut exits = Vec::with_capacity(2);
    let forward_spec = def.names.first().cloned().unwrap_or_default();
    let forward = open_exit(ctx, from, &forward_spec, to).map_err(|e| format!("{id}: {e}"))?;
    exits.push(forward);
    if let Some(expr) = lock
        && let Some(o) = ctx.w.get_mut(forward)
    {
        o.locks.insert(mush_core::lock::LockType::Basic, expr);
    }
    if let Some(back_spec) = def.names.get(1) {
        let back = open_exit(ctx, to, back_spec, from).map_err(|e| format!("{id}: {e}"))?;
        exits.push(back);
    }
    if let Some(desc) = &def.desc {
        for exit in &exits {
            ctx.w
                .set_attr(*exit, "desc", Attr::text(desc.as_str(), GOD))
                .map_err(|e| e.to_string())?;
        }
    }
    let previous = state.records.portals.get(&key).map(|p| p.revision);
    let rec = PortalRec {
        key: key.clone(),
        exits,
        from: Endpoint::Room(from),
        to: Endpoint::Room(to),
        geometry,
        mode: def.portal.mode,
        distance_m,
        dz_m: def.portal.dz_m,
        time,
        visibility: def.portal.visibility,
        revision: revision_after(previous),
    };
    rec.validate().map_err(|e| format!("{id}: {e}"))?;
    put_doc(ctx, record_key(KIND_PORTAL, &key), &rec);
    state.records.portals.insert(key, rec);
    Ok(forward)
}

// ---------------------------------------------------------------------------
// Retire
// ---------------------------------------------------------------------------

fn destroy(ctx: &mut Ctx<'_>, obj: ObjId) {
    match ctx.w.destroy(obj) {
        Ok(husks) => {
            for husk in husks {
                ctx.emit_world_event(&WorldEvent::ObjDestroyed { obj: husk });
            }
        }
        Err(error) => {
            tracing::warn!(obj = %obj, %error, "atlas: retire could not destroy an object");
        }
    }
}

fn retire_check_entry(
    kind: &str,
    ctx: &Ctx<'_>,
    state: &AtlasState,
    key: &RecordKey,
    existing: Option<ObjId>,
    placed: &BTreeMap<CanonicalId, ObjId>,
) -> Result<(), String> {
    let r = &state.records;
    let other_pack = |k: &RecordKey| k.pack != key.pack;
    match kind {
        "region" => {
            if let Some(l) = r
                .localities
                .values()
                .find(|l| &l.region == key && other_pack(&l.key))
            {
                return Err(format!("locality {} of another pack lies in it", l.key));
            }
            if let Some(n) = r
                .neighborhoods
                .values()
                .find(|n| &n.region == key && other_pack(&n.key))
            {
                return Err(format!("neighborhood {} of another pack lies in it", n.key));
            }
            if let Some(c) = r
                .connections
                .values()
                .find(|c| &c.region == key && other_pack(&c.key))
            {
                return Err(format!("connection {} of another pack crosses it", c.key));
            }
            Ok(())
        }
        "locality" => {
            if let Some(n) = r
                .neighborhoods
                .values()
                .find(|n| &n.locality == key && other_pack(&n.key))
            {
                return Err(format!("neighborhood {} of another pack lies in it", n.key));
            }
            Ok(())
        }
        "checkpoint" => {
            if let Some(c) = r
                .connections
                .values()
                .find(|c| c.legs.iter().any(|l| &l.to == key) && other_pack(&c.key))
            {
                return Err(format!("connection {} of another pack passes it", c.key));
            }
            Ok(())
        }
        "neighborhood" => {
            if let Some(s) = r
                .sites
                .values()
                .find(|s| s.neighborhoods.contains(key) && other_pack(&s.key))
            {
                return Err(format!("site {} of another pack lies in it", s.key));
            }
            if let Some(g) = r
                .roomgeo
                .values()
                .find(|g| &g.neighborhood == key && other_pack(&g.key))
            {
                return Err(format!("room {} of another pack lies in it", g.key));
            }
            Ok(())
        }
        "site" => {
            if let Some(s) = r
                .structures
                .values()
                .find(|s| &s.site == key && other_pack(&s.key))
            {
                return Err(format!("structure {} of another pack stands on it", s.key));
            }
            if let Some(g) = r
                .roomgeo
                .values()
                .find(|g| g.site.as_ref() == Some(key) && other_pack(&g.key))
            {
                return Err(format!("room {} of another pack is in it", g.key));
            }
            Ok(())
        }
        "structure" => {
            if let Some(g) = r.roomgeo.values().find(|g| {
                matches!(&g.surface, SurfaceRef::Floor { structure, .. } if structure == key)
                    && other_pack(&g.key)
            }) {
                return Err(format!(
                    "room {} of another pack is on one of its floors",
                    g.key
                ));
            }
            Ok(())
        }
        "room" => {
            let Some(room) = existing else {
                return Ok(());
            };
            let Some(obj) = ctx.w.get(room) else {
                return Ok(());
            };
            ctx.reg.check_destroy(ctx.w, GOD, room).map_err(|reason| {
                format!("{}: {reason}", mush_core::output::display_name(ctx.w, room))
            })?;
            for content in &obj.contents {
                let Some(c) = ctx.w.get(*content) else {
                    continue;
                };
                let made_here = placed.values().any(|o| o == content);
                if c.kind == ObjKind::Player {
                    return Err(format!(
                        "{} is occupied by {}",
                        obj.name,
                        mush_core::output::display_name(ctx.w, *content)
                    ));
                }
                if !made_here {
                    return Err(format!(
                        "{} holds {}, which this pack did not put there",
                        obj.name,
                        mush_core::output::display_name(ctx.w, *content)
                    ));
                }
            }
            // An exit from outside the pack leads here.
            let own_exits: Vec<ObjId> = r
                .portals
                .values()
                .filter(|p| p.key.pack == key.pack)
                .flat_map(|p| p.exits.iter().copied())
                .collect();
            if let Some(exit) = ctx.w.iter().find(|o| {
                o.kind == ObjKind::Exit && o.destination == Some(room) && !own_exits.contains(&o.id)
            }) {
                return Err(format!(
                    "exit {} ({}) from {} leads into {}",
                    exit.name,
                    exit.id,
                    exit.location.map_or_else(
                        || "nowhere".to_string(),
                        |l| mush_core::output::display_name(ctx.w, l)
                    ),
                    obj.name
                ));
            }
            Ok(())
        }
        "item" => {
            let Some(thing) = existing else {
                return Ok(());
            };
            if ctx.w.get(thing).is_none() {
                return Ok(());
            }
            ctx.reg.check_destroy(ctx.w, GOD, thing).map_err(|reason| {
                format!(
                    "{}: {reason}",
                    mush_core::output::display_name(ctx.w, thing)
                )
            })?;
            if ctx.w.get(thing).is_some_and(|o| !o.contents.is_empty()) {
                return Err(format!(
                    "{} holds things",
                    mush_core::output::display_name(ctx.w, thing)
                ));
            }
            Ok(())
        }
        "exit" => {
            let exits: Vec<ObjId> = r
                .portals
                .get(key)
                .map(|p| p.exits.clone())
                .into_iter()
                .flatten()
                .chain(existing)
                .collect();
            for exit in exits {
                if ctx.w.get(exit).is_some() {
                    ctx.reg.check_destroy(ctx.w, GOD, exit).map_err(|reason| {
                        format!("{}: {reason}", mush_core::output::display_name(ctx.w, exit))
                    })?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn retire_entry(
    kind: &str,
    ctx: &mut Ctx<'_>,
    state: &mut AtlasState,
    key: &RecordKey,
    existing: Option<ObjId>,
) -> Result<(), String> {
    let r = &mut state.records;
    match kind {
        "region" => {
            r.regions.remove(key);
            del_doc(ctx, record_key(KIND_REGION, key));
        }
        "locality" => {
            r.localities.remove(key);
            del_doc(ctx, record_key(KIND_LOCALITY, key));
        }
        "checkpoint" => {
            r.checkpoints.remove(key);
            del_doc(ctx, record_key(KIND_CHECKPOINT, key));
        }
        "connection" => {
            r.connections.remove(key);
            del_doc(ctx, record_key(KIND_CONNECTION, key));
        }
        "neighborhood" => {
            if let Some(n) = r.neighborhoods.remove(key) {
                for tile in n.tiles {
                    r.tiles.remove(&tile);
                    del_doc(ctx, record_key(KIND_TILE, &tile));
                }
            }
            del_doc(ctx, record_key(KIND_NEIGHBORHOOD, key));
        }
        "site" => {
            r.sites.remove(key);
            del_doc(ctx, record_key(KIND_SITE, key));
        }
        "structure" => {
            if let Some(s) = r.structures.remove(key) {
                for floor in s.floors {
                    if let Some(tile) = floor.tile {
                        r.tiles.remove(&tile);
                        del_doc(ctx, record_key(KIND_TILE, &tile));
                    }
                }
                let walls: Vec<RecordKey> = r
                    .features
                    .keys()
                    .filter(|k| {
                        k.pack == key.pack
                            && k.entry
                                .as_str()
                                .starts_with(&format!("{}.wall.", key.entry))
                    })
                    .cloned()
                    .collect();
                for wall in walls {
                    r.features.remove(&wall);
                    del_doc(ctx, record_key(KIND_FEATURE, &wall));
                }
            }
            del_doc(ctx, record_key(KIND_STRUCTURE, key));
        }
        "room" => {
            if let Some(room) = existing {
                r.roomgeo.remove(&room);
                del_doc(ctx, roomgeo_key(room));
                destroy(ctx, room);
            }
        }
        "feature" => {
            r.features.remove(key);
            del_doc(ctx, record_key(KIND_FEATURE, key));
        }
        "item" => {
            if let Some(thing) = existing {
                destroy(ctx, thing);
            }
        }
        "exit" => {
            let exits: Vec<ObjId> = r
                .portals
                .remove(key)
                .map(|p| p.exits)
                .into_iter()
                .flatten()
                .chain(existing)
                .collect();
            del_doc(ctx, record_key(KIND_PORTAL, key));
            let mut done: Vec<ObjId> = Vec::new();
            for exit in exits {
                if done.contains(&exit) {
                    continue;
                }
                done.push(exit);
                if ctx.w.get(exit).is_some_and(|o| o.kind == ObjKind::Exit) {
                    destroy(ctx, exit);
                }
            }
        }
        "hook" => {
            r.hooks.remove(key);
            del_doc(ctx, record_key(KIND_HOOK, key));
        }
        "secret" => {
            r.secrets.remove(key);
            del_doc(ctx, record_key(KIND_SECRET, key));
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Finish: the rasterizer, once every entry is in place
// ---------------------------------------------------------------------------

fn tile_key(owner: &RecordKey, n: usize) -> Result<RecordKey, String> {
    let entry = CanonicalId::new(format!("{}.tile.{n}", owner.entry)).map_err(|e| e.to_string())?;
    Ok(RecordKey::new(owner.pack.clone(), entry))
}

/// Drop the tiles `owner` made before, so a re-apply replaces rather than accumulates.
fn drop_tiles_of(ctx: &mut Ctx<'_>, state: &mut AtlasState, owner: &RecordKey) -> u64 {
    let prefix = format!("{}.tile.", owner.entry);
    let old: Vec<RecordKey> = state
        .records
        .tiles
        .keys()
        .filter(|k| k.pack == owner.pack && k.entry.as_str().starts_with(&prefix))
        .cloned()
        .collect();
    let mut revision = 0;
    for key in old {
        if let Some(t) = state.records.tiles.remove(&key) {
            revision = revision.max(t.revision);
        }
        del_doc(ctx, record_key(KIND_TILE, &key));
    }
    revision
}

fn finish_region(ctx: &mut Ctx<'_>, state: &mut AtlasState, pack: &Pack) -> Result<(), String> {
    let kind = CanonicalId::new("region").map_err(|e| e.to_string())?;
    let pack_id = pack.manifest().id().clone();
    let keys: Vec<RecordKey> = pack
        .entries_of(&kind)
        .map(|(id, _)| RecordKey::new(pack_id.clone(), id.clone()))
        .collect();
    for key in keys {
        let Some(region) = state.records.regions.get(&key).cloned() else {
            continue;
        };
        let bbox = region.extent.to_cm().map_err(|e| e.to_string())?;
        let previous = drop_tiles_of(ctx, state, &key);
        let input = SurfaceInput {
            region: &region,
            surface: &SurfaceRef::Ground,
            bbox,
            resolution_cm: region.ground_resolution_cm,
            rooms: Vec::new(),
            structures: Vec::new(),
            features: Vec::new(),
            portals: Vec::new(),
            floor_z_cm: 0,
            max_cells: state.limits.tile_max_cells,
        };
        let out = raster::rasterize(&input).map_err(|e| format!("{key}: ground: {e}"))?;
        let tkey = tile_key(&key, 0)?;
        let tile = TileRec {
            key: tkey.clone(),
            region: key.clone(),
            surface: SurfaceRef::Ground,
            owner: TileOwner::Region,
            bbox: out.grid.bbox().to_metres(),
            resolution_cm: out.grid.resolution_cm,
            cells: out.cells,
            rooms: out.rooms,
            revision: previous.saturating_add(1),
        };
        tile.validate().map_err(|e| format!("{tkey}: {e}"))?;
        put_doc(ctx, record_key(KIND_TILE, &tkey), &tile);
        state.records.tiles.insert(tkey.clone(), tile);
        if let Some(r) = state.records.regions.get_mut(&key) {
            r.ground = vec![tkey];
            let doc = r.clone();
            put_doc(ctx, record_key(KIND_REGION, &key), &doc);
        }
    }
    Ok(())
}

/// Sites are tactical-ready once their rooms and portals all carry geometry — which,
/// in S1, every applied one does.
fn finish_sites(ctx: &mut Ctx<'_>, state: &mut AtlasState, pack: &Pack) -> Result<(), String> {
    let pack_id = pack.manifest().id().clone();
    let keys: Vec<RecordKey> = state
        .records
        .sites
        .keys()
        .filter(|k| k.pack == pack_id)
        .cloned()
        .collect();
    for key in keys {
        let rooms_ready = state
            .records
            .roomgeo
            .values()
            .filter(|g| g.site.as_ref() == Some(&key))
            .all(|g| g.footprint.shape().is_ok());
        let entrances_ready = state.records.sites.get(&key).is_some_and(|s| {
            s.entrances
                .iter()
                .all(|e| state.records.portals.contains_key(e))
        });
        if let Some(site) = state.records.sites.get_mut(&key) {
            let ready = rooms_ready && entrances_ready;
            if site.tactical_ready != ready {
                site.tactical_ready = ready;
                let doc = site.clone();
                put_doc(ctx, record_key(KIND_SITE, &key), &doc);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn finish_neighborhood(
    ctx: &mut Ctx<'_>,
    state: &mut AtlasState,
    pack: &Pack,
) -> Result<(), String> {
    let nb_key = match own_neighborhood(state, pack) {
        Ok(k) => k,
        Err(_) => return Ok(()),
    };
    let Some(nb) = state.records.neighborhoods.get(&nb_key).cloned() else {
        return Ok(());
    };
    let Some(region) = state.records.regions.get(&nb.region).cloned() else {
        return Err(format!("{nb_key}: its region is not applied"));
    };
    let pack_id = pack.manifest().id().clone();
    let previous = drop_tiles_of(ctx, state, &nb_key);
    let mut tiles: Vec<RecordKey> = Vec::new();
    let mut n = 0_usize;

    // Ground.
    {
        let bbox = nb.footprint.to_cm().map_err(|e| e.to_string())?.bbox();
        let structures: Vec<&StructureRec> = state
            .records
            .structures
            .values()
            .filter(|s| {
                state
                    .records
                    .sites
                    .get(&s.site)
                    .is_some_and(|site| site.neighborhoods.contains(&nb_key))
            })
            .collect();
        let rooms: Vec<&RoomGeo> = state
            .records
            .rooms_in(&nb_key)
            .filter(|g| g.surface == SurfaceRef::Ground)
            .collect();
        let features: Vec<&FeatureRec> = state
            .records
            .features
            .values()
            .filter(|f| f.key.pack == pack_id && f.surface == SurfaceRef::Ground)
            .collect();
        let ground_rooms: Vec<ObjId> = rooms.iter().map(|g| g.room).collect();
        let portals: Vec<&PortalRec> = state
            .records
            .portals
            .values()
            .filter(|p| {
                [p.from, p.to]
                    .iter()
                    .any(|e| matches!(e, Endpoint::Room(r) if ground_rooms.contains(r)))
            })
            .collect();
        let input = SurfaceInput {
            region: &region,
            surface: &SurfaceRef::Ground,
            bbox,
            resolution_cm: nb.resolution_cm,
            rooms,
            structures,
            features,
            portals,
            floor_z_cm: 0,
            max_cells: state.limits.tile_max_cells,
        };
        let out = raster::rasterize(&input).map_err(|e| format!("{nb_key}: ground: {e}"))?;
        let tkey = tile_key(&nb_key, n)?;
        n += 1;
        let tile = TileRec {
            key: tkey.clone(),
            region: nb.region.clone(),
            surface: SurfaceRef::Ground,
            owner: TileOwner::Neighborhood,
            bbox: out.grid.bbox().to_metres(),
            resolution_cm: out.grid.resolution_cm,
            cells: out.cells,
            rooms: out.rooms,
            revision: previous.saturating_add(1),
        };
        tile.validate().map_err(|e| format!("{tkey}: {e}"))?;
        put_doc(ctx, record_key(KIND_TILE, &tkey), &tile);
        state.records.tiles.insert(tkey.clone(), tile);
        tiles.push(tkey);
    }

    // Each floor of each structure this pack made.
    let structure_keys: Vec<RecordKey> = state
        .records
        .structures
        .keys()
        .filter(|k| k.pack == pack_id)
        .cloned()
        .collect();
    for skey in structure_keys {
        let Some(structure) = state.records.structures.get(&skey).cloned() else {
            continue;
        };
        let bbox = structure
            .footprint
            .to_cm()
            .map_err(|e| e.to_string())?
            .bbox();
        let mut floors = structure.floors.clone();
        for floor in &mut floors {
            let surface = SurfaceRef::Floor {
                structure: skey.clone(),
                index: floor.index,
            };
            let rooms: Vec<&RoomGeo> = state
                .records
                .roomgeo
                .values()
                .filter(|g| g.surface == surface)
                .collect();
            let floor_rooms: Vec<ObjId> = rooms.iter().map(|g| g.room).collect();
            let features: Vec<&FeatureRec> = state
                .records
                .features
                .values()
                .filter(|f| f.surface == surface)
                .collect();
            let portals: Vec<&PortalRec> = state
                .records
                .portals
                .values()
                .filter(|p| {
                    [p.from, p.to]
                        .iter()
                        .any(|e| matches!(e, Endpoint::Room(r) if floor_rooms.contains(r)))
                })
                .collect();
            let floor_z_cm = i32::try_from(cm(floor.z_offset_m).map_err(|e| e.to_string())?)
                .map_err(|_| format!("{skey}: floor {} is out of range", floor.index))?;
            let input = SurfaceInput {
                region: &region,
                surface: &surface,
                bbox,
                resolution_cm: nb.resolution_cm,
                rooms,
                structures: vec![&structure],
                features,
                portals,
                floor_z_cm,
                max_cells: state.limits.tile_max_cells,
            };
            let out = raster::rasterize(&input)
                .map_err(|e| format!("{skey}: floor {}: {e}", floor.index))?;
            let tkey = tile_key(&nb_key, n)?;
            n += 1;
            let tile = TileRec {
                key: tkey.clone(),
                region: nb.region.clone(),
                surface: surface.clone(),
                owner: TileOwner::Structure,
                bbox: out.grid.bbox().to_metres(),
                resolution_cm: out.grid.resolution_cm,
                cells: out.cells,
                rooms: out.rooms,
                revision: previous.saturating_add(1),
            };
            tile.validate().map_err(|e| format!("{tkey}: {e}"))?;
            put_doc(ctx, record_key(KIND_TILE, &tkey), &tile);
            state.records.tiles.insert(tkey.clone(), tile);
            floor.tile = Some(tkey.clone());
            tiles.push(tkey);
        }
        if let Some(s) = state.records.structures.get_mut(&skey) {
            s.floors = floors;
            let doc = s.clone();
            put_doc(ctx, record_key(KIND_STRUCTURE, &skey), &doc);
        }
    }
    if tiles.len() > crate::record::MAX_TILES_PER_NEIGHBORHOOD {
        return Err(format!(
            "{nb_key}: {} tiles, over {}",
            tiles.len(),
            crate::record::MAX_TILES_PER_NEIGHBORHOOD
        ));
    }
    if let Some(n) = state.records.neighborhoods.get_mut(&nb_key) {
        n.tiles = tiles;
        let doc = n.clone();
        put_doc(ctx, record_key(KIND_NEIGHBORHOOD, &nb_key), &doc);
    }
    Ok(())
}

/// The point a portal is drawn at, from its shared threshold (re-exported for the
/// views).
#[must_use]
pub fn portal_point(threshold: (PointCm, PointCm)) -> PointCm {
    threshold_point(threshold)
}
