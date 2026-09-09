//! A **place** is any of the atlas's containers a staffer can name — a region, a
//! locality, a neighborhood or a site — plus the world itself (S2b, plan §3.3).
//! Themes hang off places, weather cells key on neighborhoods and regions, and the
//! faction and housing modules point their records at places; this is the one
//! lookup they all share.

use mush_core::{CanonicalId, ObjId, World};

use crate::geom::Polygon2;
use crate::module::AtlasState;
use crate::record::{RecordKey, Records};

/// One of the atlas's containers, or the world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceRef {
    /// The world: the default everything falls to.
    World,
    /// A region.
    Region(RecordKey),
    /// A locality.
    Locality(RecordKey),
    /// A neighborhood.
    Neighborhood(RecordKey),
    /// A site.
    Site(RecordKey),
}

impl PlaceRef {
    /// The record key, if any.
    #[must_use]
    pub fn key(&self) -> Option<&RecordKey> {
        match self {
            PlaceRef::World => None,
            PlaceRef::Region(k)
            | PlaceRef::Locality(k)
            | PlaceRef::Neighborhood(k)
            | PlaceRef::Site(k) => Some(k),
        }
    }

    /// The kind word.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            PlaceRef::World => "world",
            PlaceRef::Region(_) => "region",
            PlaceRef::Locality(_) => "locality",
            PlaceRef::Neighborhood(_) => "neighborhood",
            PlaceRef::Site(_) => "site",
        }
    }

    /// The name shown.
    #[must_use]
    pub fn name(&self, records: &Records) -> String {
        match self {
            PlaceRef::World => Some("the world".to_string()),
            PlaceRef::Region(k) => records.regions.get(k).map(|r| r.name.clone()),
            PlaceRef::Locality(k) => records.localities.get(k).map(|r| r.name.clone()),
            PlaceRef::Neighborhood(k) => records.neighborhoods.get(k).map(|r| r.name.clone()),
            PlaceRef::Site(k) => records.sites.get(k).map(|r| r.name.clone()),
        }
        .unwrap_or_else(|| "a place that is gone".to_string())
    }

    /// The id a staffer types to name it again: the pack id for a region or a
    /// neighborhood (one per pack), `<pack>:<entry>` otherwise.
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            PlaceRef::World => "world".to_string(),
            PlaceRef::Region(k) | PlaceRef::Neighborhood(k) => k.pack.to_string(),
            PlaceRef::Locality(k) | PlaceRef::Site(k) => format!("{}:{}", k.pack, k.entry),
        }
    }

    /// The footprint, region metres, when the place has one.
    #[must_use]
    pub fn footprint(&self, records: &Records) -> Option<Polygon2> {
        match self {
            PlaceRef::World | PlaceRef::Region(_) => None,
            PlaceRef::Locality(k) => records.localities.get(k).map(|r| r.footprint.clone()),
            PlaceRef::Neighborhood(k) => records.neighborhoods.get(k).map(|r| r.footprint.clone()),
            PlaceRef::Site(k) => records.sites.get(k).map(|r| r.footprint.clone()),
        }
    }

    /// The extent as `(width_m, height_m, x0, y0)`, from the footprint or a region's
    /// extent.
    #[must_use]
    pub fn extent_m(&self, records: &Records) -> Option<(f64, f64, f64, f64)> {
        if let PlaceRef::Region(k) = self {
            let r = records.regions.get(k)?;
            let (a, b) = (r.extent.0, r.extent.1);
            return Some((
                (b.0 - a.0).abs(),
                (b.1 - a.1).abs(),
                a.0.min(b.0),
                a.1.min(b.1),
            ));
        }
        let poly = self.footprint(records)?;
        let xs = poly.0.iter().map(|p| p.0);
        let ys = poly.0.iter().map(|p| p.1);
        let (x0, x1) = (
            xs.clone().fold(f64::INFINITY, f64::min),
            xs.fold(f64::NEG_INFINITY, f64::max),
        );
        let (y0, y1) = (
            ys.clone().fold(f64::INFINITY, f64::min),
            ys.fold(f64::NEG_INFINITY, f64::max),
        );
        if !x0.is_finite() || !y0.is_finite() {
            return None;
        }
        Some((x1 - x0, y1 - y0, x0, y0))
    }

    /// The place one tier up: site → neighborhood → locality → region → world.
    #[must_use]
    pub fn parent(&self, records: &Records) -> Option<PlaceRef> {
        match self {
            PlaceRef::World => None,
            PlaceRef::Region(_) => Some(PlaceRef::World),
            PlaceRef::Locality(k) => records
                .localities
                .get(k)
                .map(|l| PlaceRef::Region(l.region.clone())),
            PlaceRef::Neighborhood(k) => records
                .neighborhoods
                .get(k)
                .map(|n| PlaceRef::Locality(n.locality.clone())),
            PlaceRef::Site(k) => records
                .sites
                .get(k)
                .and_then(|s| s.neighborhoods.first().cloned())
                .map(PlaceRef::Neighborhood),
        }
    }
}

/// The places a room resolves through, nearest first: its site (if any), its
/// neighborhood, the locality, the region, the world. A room off the map resolves
/// through the world alone.
#[must_use]
pub fn chain_for_room(records: &Records, room: ObjId) -> Vec<PlaceRef> {
    let mut chain = Vec::new();
    if let Some(geo) = records.roomgeo.get(&room) {
        if let Some(site) = &geo.site {
            chain.push(PlaceRef::Site(site.clone()));
        }
        let mut cur = Some(PlaceRef::Neighborhood(geo.neighborhood.clone()));
        while let Some(p) = cur {
            let next = p.parent(records);
            chain.push(p);
            cur = next;
            if chain.len() > 8 {
                break;
            }
        }
    }
    if !chain.contains(&PlaceRef::World) {
        chain.push(PlaceRef::World);
    }
    chain
}

/// The places a place resolves through, itself first.
#[must_use]
pub fn chain_for_place(records: &Records, place: &PlaceRef) -> Vec<PlaceRef> {
    let mut chain = vec![place.clone()];
    let mut cur = place.parent(records);
    while let Some(p) = cur {
        if chain.contains(&p) || chain.len() > 8 {
            break;
        }
        cur = p.parent(records);
        chain.push(p);
    }
    if !chain.contains(&PlaceRef::World) {
        chain.push(PlaceRef::World);
    }
    chain
}

/// The neighborhood a room is in, if it is on the map.
#[must_use]
pub fn neighborhood_of_room(records: &Records, room: ObjId) -> Option<RecordKey> {
    records.roomgeo.get(&room).map(|g| g.neighborhood.clone())
}

/// The region a neighborhood is in.
#[must_use]
pub fn region_of_neighborhood(records: &Records, neighborhood: &RecordKey) -> Option<RecordKey> {
    records
        .neighborhoods
        .get(neighborhood)
        .map(|n| n.region.clone())
}

/// A neighborhood by its pack id.
#[must_use]
pub fn neighborhood_by_pack(records: &Records, pack: &CanonicalId) -> Option<RecordKey> {
    records
        .neighborhoods
        .values()
        .find(|n| &n.key.pack == pack)
        .map(|n| n.key.clone())
}

/// A region by its pack id.
#[must_use]
pub fn region_by_pack(records: &Records, pack: &CanonicalId) -> Option<RecordKey> {
    records
        .regions
        .values()
        .find(|r| &r.key.pack == pack)
        .map(|r| r.key.clone())
}

fn parse_key(s: &str) -> Option<RecordKey> {
    let (pack, entry) = s.split_once(':')?;
    Some(RecordKey::new(
        CanonicalId::new(pack.trim()).ok()?,
        CanonicalId::new(entry.trim()).ok()?,
    ))
}

/// Find a place by what a staffer typed: `world` (or `#0`, `default`), a pack id
/// (`region.limbo`, `neighborhood.ooc_area`), `<pack>:<entry>` for a locality or a
/// site, or a name — exact first, then a unique prefix, then a unique substring —
/// across regions, localities, neighborhoods and sites in that order.
#[must_use]
pub fn find_place(records: &Records, query: &str) -> Option<PlaceRef> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    if q.eq_ignore_ascii_case("world") || q == "#0" || q.eq_ignore_ascii_case("default") {
        return Some(PlaceRef::World);
    }
    if let Some(key) = parse_key(q) {
        if records.localities.contains_key(&key) {
            return Some(PlaceRef::Locality(key));
        }
        if records.sites.contains_key(&key) {
            return Some(PlaceRef::Site(key));
        }
        if records.regions.contains_key(&key) {
            return Some(PlaceRef::Region(key));
        }
        if records.neighborhoods.contains_key(&key) {
            return Some(PlaceRef::Neighborhood(key));
        }
    }
    if let Ok(id) = CanonicalId::new(q) {
        if let Some(k) = region_by_pack(records, &id) {
            return Some(PlaceRef::Region(k));
        }
        if let Some(k) = neighborhood_by_pack(records, &id) {
            return Some(PlaceRef::Neighborhood(k));
        }
    }
    let all: Vec<(String, PlaceRef)> = records
        .regions
        .values()
        .map(|r| (r.name.clone(), PlaceRef::Region(r.key.clone())))
        .chain(
            records
                .localities
                .values()
                .map(|l| (l.name.clone(), PlaceRef::Locality(l.key.clone()))),
        )
        .chain(
            records
                .neighborhoods
                .values()
                .map(|n| (n.name.clone(), PlaceRef::Neighborhood(n.key.clone()))),
        )
        .chain(
            records
                .sites
                .values()
                .map(|s| (s.name.clone(), PlaceRef::Site(s.key.clone()))),
        )
        .collect();
    let needle = q.to_ascii_lowercase();
    if let Some((_, p)) = all.iter().find(|(n, _)| n.to_ascii_lowercase() == needle) {
        return Some(p.clone());
    }
    let prefix: Vec<&(String, PlaceRef)> = all
        .iter()
        .filter(|(n, _)| n.to_ascii_lowercase().starts_with(&needle))
        .collect();
    if let [one] = prefix.as_slice() {
        return Some(one.1.clone());
    }
    let within: Vec<&(String, PlaceRef)> = all
        .iter()
        .filter(|(n, _)| n.to_ascii_lowercase().contains(&needle))
        .collect();
    if let [one] = within.as_slice() {
        return Some(one.1.clone());
    }
    None
}

/// The records, from the world's atlas slot.
#[must_use]
pub fn records(w: &World) -> Option<&Records> {
    w.module_state::<AtlasState>().map(|st| &st.records)
}
