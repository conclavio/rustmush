//! **S9.1 (D-14-33)** — a campaign-seed pack's places and routes are the atlas's
//! records, applied through `mush-content`'s latch like any other pack.
//!
//! A `location` entry makes a room (as the S1–S7 seed did) **and** a
//! [`SiteRec`] in the pack's region — an orbital or system region, where a
//! site lies in no neighborhood and is anchored by the region alone — positioned
//! from the campaign ephemeris at the epoch, so that a `route` entry's
//! [`ConnectionRec`], drawn straight between its two places, is as long as the
//! sky's pinned distance by construction: one number, one place. The other
//! kinds a TacSim pack carries (commodities, recipes, deposits, facilities,
//! formations, bodies) are definitions the [`TacsimStore`] reads from the pack
//! itself; their applier makes no object.
//!
//! The content latch is the one latch: a re-applied version updates in place
//! (the room keeps its id; the records their keys, revision up), a retired
//! pack takes its records back and leaves the rooms standing.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_atlas::geom::{Point2, Point3, Polygon2};
use mush_atlas::record::{
    AccessPoint, ConnectionKind, ConnectionRec, ConnectionStatus, Direction, LegDef, RecordKey,
    ReviewState, SiteOrigin, SiteRec, Visibility,
};
use mush_content::{Appliers, EntryApplier, Pack, PackError, RawEntry};
use mush_core::{CanonicalId, Ctx, ObjId, ObjKind};

use crate::ephemeris::{AU_KM, Body, Ephemeris};
use crate::pack::{BodyDef, LocationDef, RouteEntry};
use crate::store::TacsimStore;

/// The region unit a system region is drawn in: gigametres, so an orbit fits
/// the atlas's coordinate range and a site is still a real footprint.
pub const GM_PER_KM: f64 = 1.0e-6;

/// Half the side of an orbital site's footprint in the system frame's units
/// (gigametres): a site is a marker on a map of orbits, not a footprint to walk,
/// and the atlas's geometry (which counts a region unit as a metre for its own
/// purposes) wants a box that is not degenerate at its centimetre grain.
pub const SITE_HALF_GM: f64 = 0.5;

/// The mode word a corridor carries: what travels it.
pub const CORRIDOR_MODE: &str = "spacecraft";

/// The kinds a campaign-seed pack carries that make no object of their own.
const DEFINITION_KINDS: [&str; 6] = [
    "commodity",
    "recipe",
    "deposit",
    "facility",
    "formation",
    "body",
];

/// Register this crate's appliers: places, routes, and the definition kinds.
///
/// # Errors
///
/// A kind another owner already claimed.
pub fn register_appliers(appliers: &mut Appliers) -> Result<(), PackError> {
    appliers.register(crate::NS, Arc::new(PlaceApplier))?;
    appliers.register(crate::NS, Arc::new(RouteApplier))?;
    appliers.register(crate::NS, Arc::new(DefinitionApplier))
}

/// A kind id from one of this crate's static kind words; `None` only if the
/// word itself were malformed, which the contract test over `OWNED_KINDS` keeps
/// from ever being true.
fn kind(name: &str) -> Option<CanonicalId> {
    CanonicalId::new(name).ok()
}

/// The pack's one `region` entry — a campaign-seed pack declares the orbital or
/// system region its places lie in.
fn region_of(pack: &Pack) -> Result<RecordKey, String> {
    let Some(region) = kind("region") else {
        return Err("the region kind word is malformed".to_string());
    };
    let mut regions = pack.entries_of(&region);
    let Some((id, _)) = regions.next() else {
        return Err(format!(
            "{}: a campaign-seed pack declares one `region` entry its places lie in",
            pack.manifest().id()
        ));
    };
    if regions.next().is_some() {
        return Err(format!(
            "{}: a campaign-seed pack declares one region, not several",
            pack.manifest().id()
        ));
    }
    Ok(RecordKey::new(pack.manifest().id().clone(), id.clone()))
}

/// The sky the pack's `body` entries describe, at the store's configured epoch.
fn sky_of(ctx: &Ctx<'_>, pack: &Pack) -> Ephemeris {
    let cfg = ctx
        .state::<TacsimStore>()
        .map(|s| s.config().time.ephemeris.clone())
        .unwrap_or_default();
    let mut sky = Ephemeris::new(&cfg);
    let Some(body) = kind("body") else {
        return sky;
    };
    for (id, entry) in pack.entries_of(&body) {
        if let Ok(def) = entry.decode::<BodyDef>() {
            sky = sky.with_body(id.clone(), Body::from(&def));
        }
    }
    sky
}

/// Where a body stands at the epoch, gigametres in the system frame: a circular
/// orbit of `orbit_uau` micro-AU at the epoch phase.
fn position_gm(sky: &Ephemeris, body: &CanonicalId) -> Option<Point2> {
    let b = sky.body(body)?;
    let phase_mdeg = sky.phase_mdeg_at(body, sky.epoch())?;
    #[allow(clippy::cast_precision_loss)]
    let radius_gm = (b.orbit_uau as f64) / 1_000_000.0 * (AU_KM as f64) * GM_PER_KM;
    #[allow(clippy::cast_precision_loss)]
    let phase = (phase_mdeg as f64 / 1000.0).to_radians();
    Some(Point2(radius_gm * phase.cos(), radius_gm * phase.sin()))
}

/// A place's position: its body's, or the frame's origin for a place the sky
/// does not know.
fn place_position(sky: &Ephemeris, def: &LocationDef) -> Point2 {
    def.body
        .as_ref()
        .and_then(|b| position_gm(sky, b))
        .unwrap_or(Point2(0.0, 0.0))
}

fn footprint_at(p: Point2) -> Polygon2 {
    Polygon2(vec![
        Point2(p.0 - SITE_HALF_GM, p.1 - SITE_HALF_GM),
        Point2(p.0 + SITE_HALF_GM, p.1 - SITE_HALF_GM),
        Point2(p.0 + SITE_HALF_GM, p.1 + SITE_HALF_GM),
        Point2(p.0 - SITE_HALF_GM, p.1 + SITE_HALF_GM),
    ])
}

/// A `location`: a room, and a site in the pack's region.
struct PlaceApplier;

impl PlaceApplier {
    fn put(
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        existing: Option<ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let def: LocationDef = entry.decode().map_err(|e| format!("{id}: {e}"))?;
        let region = region_of(pack)?;
        let room = match existing {
            Some(room) if ctx.w.get(room).is_some() => {
                if ctx.w.get(room).is_some_and(|o| o.name != def.label) {
                    ctx.w
                        .set_name(room, &def.label)
                        .map_err(|e| format!("{id}: {e}"))?;
                }
                room
            }
            _ => {
                let owner = ctx.actor;
                ctx.w
                    .create(ObjKind::Room, &def.label, owner, None)
                    .map_err(|e| format!("{id}: {e}"))?
            }
        };
        let sky = sky_of(ctx, pack);
        let at = place_position(&sky, &def);
        let key = RecordKey::new(pack.manifest().id().clone(), id.clone());
        let rec = SiteRec {
            key,
            neighborhoods: Vec::new(),
            region: Some(region),
            name: def.label.clone(),
            kind: format!("site.{}", def.role),
            footprint: footprint_at(at),
            entrances: Vec::new(),
            property: None,
            origin: SiteOrigin::Authored,
            review: ReviewState::NotApplicable,
            tactical_ready: false,
            theme: None,
            revision: 0,
        };
        mush_atlas::put_site(ctx, rec)?;
        Ok(Some(room))
    }
}

impl EntryApplier for PlaceApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        kind("location").into_iter().collect()
    }

    fn phase(&self) -> u8 {
        // After the atlas's region (0), before the routes between the places.
        5
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        PlaceApplier::put(ctx, pack, id, entry, None)
    }

    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        // In place: the room keeps its id and everything standing in it.
        PlaceApplier::put(ctx, pack, id, entry, existing)
    }

    fn retire_check(
        &self,
        _ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        _existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn retire(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
    ) -> Result<(), String> {
        // The record goes; the room stands (it may hold people and things).
        let key = RecordKey::new(pack.clone(), id.clone());
        mush_atlas::retire_record(ctx, "site", &key)?;
        if let Some(room) = existing {
            tracing::info!(%pack, %id, room = room.0, "tacsim: a place's record retired; its room stands");
        }
        Ok(())
    }
}

/// A `route`: a connection straight between its two places.
struct RouteApplier;

impl RouteApplier {
    fn put(
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let def: RouteEntry = entry.decode().map_err(|e| format!("{id}: {e}"))?;
        let region = region_of(pack)?;
        let sky = sky_of(ctx, pack);
        let location =
            kind("location").ok_or_else(|| format!("{id}: the location kind word is malformed"))?;
        let place = |symbol: &CanonicalId| -> Result<(ObjId, Point2, String), String> {
            let room = placed
                .get(symbol)
                .copied()
                .ok_or_else(|| format!("{id}: the pack placed no object for {symbol}"))?;
            let def: LocationDef = pack
                .entry(symbol)
                .filter(|e| e.kind() == &location)
                .ok_or_else(|| format!("{id}: {symbol} is not a location of this pack"))?
                .decode()
                .map_err(|e| format!("{id}: {symbol}: {e}"))?;
            Ok((room, place_position(&sky, &def), def.label))
        };
        let (from_room, from_at, from_name) = place(&def.from)?;
        let (to_room, to_at, to_name) = place(&def.to)?;
        let key = RecordKey::new(pack.manifest().id().clone(), id.clone());
        let total_s: u32 = def
            .legs
            .iter()
            .map(|l| l.base_s)
            .fold(0u32, |acc, s| acc.saturating_add(s))
            .max(1);
        let rec = ConnectionRec {
            key: key.clone(),
            region,
            name: format!("{from_name} – {to_name}"),
            kind: ConnectionKind::Journey,
            endpoints: (
                AccessPoint {
                    neighborhood: None,
                    room: Some(from_room),
                    point: Point3(from_at.0, from_at.1, 0.0),
                },
                AccessPoint {
                    neighborhood: None,
                    room: Some(to_room),
                    point: Point3(to_at.0, to_at.1, 0.0),
                },
            ),
            direction: Direction::Both,
            modes: vec![CORRIDOR_MODE.to_string()],
            corridor: vec![
                Point3(from_at.0, from_at.1, 0.0),
                Point3(to_at.0, to_at.1, 0.0),
            ],
            legs: vec![LegDef {
                to: key,
                base_s: total_s,
                hazard: 0,
                interdiction: 0,
            }],
            cost: serde_json::json!({
                "drive_class": def.drive_class.to_string(),
                "propellant": def.propellant,
                "legs": def.legs.len(),
            }),
            visibility: Visibility::Public,
            status: ConnectionStatus::Active,
            service: None,
            revision: 0,
        };
        mush_atlas::put_connection(ctx, rec)?;
        Ok(None)
    }
}

impl EntryApplier for RouteApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        kind("route").into_iter().collect()
    }

    fn phase(&self) -> u8 {
        6
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        RouteApplier::put(ctx, pack, id, entry, placed)
    }

    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        RouteApplier::put(ctx, pack, id, entry, placed)
    }

    fn retire_check(
        &self,
        _ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        _existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn retire(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        _existing: Option<ObjId>,
    ) -> Result<(), String> {
        let key = RecordKey::new(pack.clone(), id.clone());
        mush_atlas::retire_record(ctx, "connection", &key)
    }
}

/// The definition kinds: read from the pack by the store, no object made.
struct DefinitionApplier;

impl EntryApplier for DefinitionApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        DEFINITION_KINDS.iter().filter_map(|k| kind(k)).collect()
    }

    fn phase(&self) -> u8 {
        12
    }

    fn create(
        &self,
        _ctx: &mut Ctx<'_>,
        _pack: &Pack,
        _id: &CanonicalId,
        _entry: &RawEntry,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        Ok(None)
    }

    fn update(
        &self,
        _ctx: &mut Ctx<'_>,
        _pack: &Pack,
        _id: &CanonicalId,
        _entry: &RawEntry,
        _existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        Ok(None)
    }

    fn retire_check(
        &self,
        _ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        _existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn retire(
        &self,
        _ctx: &mut Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        _existing: Option<ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }
}
