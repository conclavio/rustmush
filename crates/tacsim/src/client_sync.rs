//! **Additive structured client sync & port trade feeds** (`port.trade`,
//! `tacsim.content`).
//!
//! Two pull-only panel feeds for graphical 3D clients (such as the Unity URP
//! client), served exclusively over authenticated WebSocket `PanelSubscribe`
//! queries (working identically in all-in-one `mushd` and split
//! `mush-gateway` + `mush-server` deployments):
//!
//! - [`PORT_TRADE_FEED`] (`"port.trade"`): structured port venue (gated by
//!   [`PortVenueGate`]), actor liquid credit balance (via
//!   [`crate::port::credit::own_account`]), paginated visible market asks/bids
//!   with post-spread execution prices and funding flags (via
//!   [`mush_market::summarize_listing`]), open buyer quotes for correlated
//!   purchase confirmation, own-hull stowage gauges, and contextual berth /
//!   simulator rosters.
//! - [`CLIENT_CONTENT_FEED`] (`"tacsim.content"`): SHA-256 content manifest and
//!   player-safe projected pack JSON (`secret`, `lore`, `brief`, hidden staff
//!   exits, restricted room prose, and NPC prompt internals stripped) derived
//!   directly from canonical `mush-atlas` and `tacsim` content tables.
//!
//! Neither feed ends in a suffix auto-subscribed by the web SPA or SSH TUI
//! (`.vitals`, `.view`, `.unit`, `.sheet`, `.roll`, `.chart`, `.map`,
//! `.market`), so existing clients ignore both unless they explicitly query
//! them.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use mush_core::module::{ModError, Registrar};
use mush_core::{HookReadContext, ObjId, QueryHandler, World};
use mush_economy::{Amount, EconomyStore, InstrumentId};
use mush_market::{ListingId, MARKET_PAGE, MarketFilter, MarketStore};
use mush_org::PrincipalId;
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::definitions::content::{
    CAST_SOURCES, LANE_JSON, MARS_CANYON_ATLAS_JSON, PROVING_GROUND_ATLAS_JSON, SITE_SOURCES,
    STATION_SOURCES, WIDE_SITE_SOURCES, WINTER_VALLEY_ATLAS_JSON, fielded_sources,
};
use crate::definitions::goods::GOODS_JSON;
use crate::economy::CREDIT;
use crate::port::board::money;
use crate::port::coop::{EnrolmentStatus, helm_of, name_of, port_name};
use crate::port::credit::own_account;
use crate::port::market::{PortVenueGate, port_of_venue};
use crate::port::ship::{
    Berth, berth_gallery_json, berth_of, hull_of_room, hull_stowage_json, hull_view,
    simulator_lobby_json,
};
use crate::store::TacsimStore;

/// Pulled panel feed returning structured port market listings, actor credits,
/// open buyer quotes, stowage gauges, and contextual berth/simulator rosters.
pub const PORT_TRADE_FEED: &str = "port.trade";

/// Pulled panel feed returning the projected client content pack manifest and
/// individual player-safe pack JSON payloads by id.
pub const CLIENT_CONTENT_FEED: &str = "tacsim.content";

/// Schema version for both client feeds (`2` = player-safe projection +
/// correlated quotes + post-spread pricing + contextual rosters).
pub const SCHEMA_VERSION: u32 = 2;

const CONDUCT_CARRIER_JSON: &str = include_str!("../content/units/conduct_carrier.json");

/// Precomputed SHA-256 entry for one projected client content pack.
#[derive(Clone, Debug)]
pub struct ClientPackEntry {
    /// Canonical client pack identifier (`"neighborhood.ceres_freeport"`, `"dray"`, `"goods"`, ...).
    pub id: Arc<str>,
    /// Pack category (`"neighborhood"`, `"cast"`, `"unit"`, `"weapon"`, `"arena"`, `"arena_atlas"`, `"market"`, `"lane"`).
    pub category: &'static str,
    /// Lowercase hex SHA-256 of the projected `json`.
    pub sha256: Arc<str>,
    /// Projected player-safe UTF-8 JSON payload.
    pub json: Arc<str>,
}

/// Cached bundle of all projected client content packs and their manifest JSON.
#[derive(Debug)]
pub struct ClientContentBundle {
    /// Manifest entries as a `serde_json::Value` array for WebSocket frames.
    pub manifest_packs_value: Value,
    /// Individual projected pack entries in deterministic manifest order.
    pub entries: Vec<ClientPackEntry>,
    /// Lookup index mapping canonical IDs and aliases to index in `entries`.
    by_id: BTreeMap<String, usize>,
}

impl ClientContentBundle {
    /// Look up a projected pack entry by canonical ID or alias.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&ClientPackEntry> {
        let idx = *self.by_id.get(id.trim())?;
        self.entries.get(idx)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn public_role_clause(raw: &str) -> String {
    let trimmed = raw.trim();
    let semi = trimmed.find(';');
    let dot = trimmed.find('.');
    let cut = match (semi, dot) {
        (Some(s), Some(d)) => Some(s.min(d)),
        (Some(s), None) => Some(s),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    };
    let head = match cut {
        Some(idx) if idx > 0 && idx <= 110 => trimmed.get(..idx).unwrap_or(trimmed).trim(),
        _ => trimmed,
    };
    if head.chars().count() > 96 {
        let mut short: String = head.chars().take(93).collect();
        short.push_str("...");
        short
    } else {
        head.to_owned()
    }
}

fn is_allowed_client_entry_kind(kind: &str) -> bool {
    matches!(
        kind,
        "neighborhood"
            | "locality"
            | "site"
            | "structure"
            | "room"
            | "exit"
            | "feature"
            | "item"
            | "region"
            | "heightfield"
            | "connection"
            | "checkpoint"
            | "npc"
            | "person"
            | "unit"
            | "weapon"
            | "arena"
            | "body"
            | "location"
            | "route"
            | "port"
    )
}

fn entry_is_staff_or_hidden(kind: &str, def: Option<&Map<String, Value>>) -> bool {
    // Default-deny: drop secrets, lore, quests, hooks, deposits, facilities, crew_offers,
    // passengers, relay_networks, recipes, and any non-allowlisted entry kind.
    if !is_allowed_client_entry_kind(kind) {
        return true;
    }
    let Some(def) = def else {
        return false;
    };
    if def.get("reveal").and_then(Value::as_str) == Some("staff") {
        return true;
    }
    if def.get("visibility").and_then(Value::as_str) == Some("hidden") {
        return true;
    }
    if def
        .get("portal")
        .and_then(Value::as_object)
        .and_then(|p| p.get("visibility"))
        .and_then(Value::as_str)
        == Some("hidden")
    {
        return true;
    }
    false
}

fn sanitize_entry_in_place(kind: &str, def: &mut Map<String, Value>) {
    match kind {
        "room" => {
            if def.get("access").and_then(Value::as_str) == Some("restricted") {
                def.remove("desc");
                def.remove("details");
            }
        }
        "npc" | "person" => {
            // Strict public-surface allowlist: strips `lore` (guarded/secret facts),
            // `lore_tags`, `knowledge`, `tags`, `abilities`, `schedule`, `memory`,
            // and any non-public NPC authoring metadata.
            const ALLOWED_NPC_KEYS: &[&str] = &[
                "name",
                "aliases",
                "room",
                "home",
                "enabled",
                "short_desc",
                "desc",
                "profile",
            ];
            def.retain(|k, _| ALLOWED_NPC_KEYS.contains(&k.as_str()));
            if let Some(profile) = def.get_mut("profile").and_then(Value::as_object_mut) {
                let enabled = profile
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let bg = profile
                    .get("background")
                    .and_then(Value::as_str)
                    .map(public_role_clause)
                    .unwrap_or_default();
                profile.clear();
                profile.insert("enabled".to_owned(), Value::Bool(enabled));
                if !bg.is_empty() {
                    profile.insert("background".to_owned(), Value::String(bg));
                }
            }
        }
        "port" => {
            // Strip vendor stock, ask/bid prices, intake caps, refresh rates, treasuries,
            // org controls, and programme internals — retain public chart references only.
            const ALLOWED_PORT_KEYS: &[&str] =
                &["location", "venue", "site", "area", "gallery", "routes"];
            def.retain(|k, _| ALLOWED_PORT_KEYS.contains(&k.as_str()));
        }
        "location" => {
            // Strip `kirkwood_picket` and rock `gross_t` reserves; retain public chart fields.
            const ALLOWED_LOCATION_KEYS: &[&str] = &[
                "label", "name", "body", "role", "lat_mdeg", "lon_mdeg", "clusters",
            ];
            def.retain(|k, _| ALLOWED_LOCATION_KEYS.contains(&k.as_str()));
            if let Some(clusters) = def.get_mut("clusters").and_then(Value::as_array_mut) {
                for cluster in clusters {
                    if let Some(cobj) = cluster.as_object_mut() {
                        cobj.retain(|k, _| matches!(k.as_str(), "id" | "label" | "rocks"));
                        if let Some(rocks) = cobj.get_mut("rocks").and_then(Value::as_array_mut) {
                            for rock in rocks {
                                if let Some(robj) = rock.as_object_mut() {
                                    robj.retain(|k, _| {
                                        matches!(k.as_str(), "id" | "family" | "spectral_class")
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        "route" => {
            const ALLOWED_ROUTE_KEYS: &[&str] =
                &["from", "to", "drive_class", "propellant", "legs"];
            def.retain(|k, _| ALLOWED_ROUTE_KEYS.contains(&k.as_str()));
        }
        "body" => {
            const ALLOWED_BODY_KEYS: &[&str] = &[
                "parent",
                "orbit_au",
                "period_days",
                "phase_deg_at_epoch",
                "epoch_ic",
                "radius_km",
                "surface_mg",
            ];
            def.retain(|k, _| ALLOWED_BODY_KEYS.contains(&k.as_str()));
        }
        _ => {}
    }
}

/// Structural check for whether a neighborhood pack represents a player-accessible
/// 3D area: requires at least one public room and excludes non-OOC Limbo neighborhoods
/// (such as studio/staff backlots in `region.limbo` whose `setting != "ooc"`).
fn neighborhood_is_player_accessible(doc: &Value) -> bool {
    let Some(entries) = doc.get("entries").and_then(Value::as_object) else {
        return false;
    };
    let mut has_public_room = false;
    let mut is_non_ooc_limbo = false;
    for entry in entries.values() {
        let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
        let Some(def) = entry.get("def").and_then(Value::as_object) else {
            continue;
        };
        if kind == "room" && def.get("access").and_then(Value::as_str) != Some("restricted") {
            has_public_room = true;
        } else if kind == "neighborhood" {
            let region = def.get("region").and_then(Value::as_str).unwrap_or("");
            let setting = def.get("setting").and_then(Value::as_str).unwrap_or("");
            if region == "region.limbo" && setting != "ooc" {
                is_non_ooc_limbo = true;
            }
        }
    }
    has_public_room && !is_non_ooc_limbo
}

fn project_pack_value_for_client(mut doc: Value) -> Option<String> {
    let root = doc.as_object_mut()?;
    root.remove("brief");
    if let Some(manifest) = root.get_mut("manifest").and_then(Value::as_object_mut) {
        manifest.remove("digest");
        manifest.insert("projected".to_owned(), Value::Bool(true));
    }
    if let Some(entries) = root.get_mut("entries").and_then(Value::as_object_mut) {
        let drop_keys: Vec<String> = entries
            .iter()
            .filter_map(|(key, entry)| {
                let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
                let def = entry.get("def").and_then(Value::as_object);
                entry_is_staff_or_hidden(kind, def).then_some(key.clone())
            })
            .collect();
        for key in drop_keys {
            entries.remove(&key);
        }
        for entry in entries.values_mut() {
            let kind = entry
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if let Some(def) = entry.get_mut("def").and_then(Value::as_object_mut) {
                sanitize_entry_in_place(&kind, def);
            }
        }
    }
    serde_json::to_string(&doc).ok()
}

/// Project an authoring JSON pack down to player-safe client geometry and public
/// metadata, stripping `"brief"`, `"digest"`, `"secret"`/`"lore"`/`"quest"`
/// entries, `"reveal": "staff"`, `"visibility": "hidden"` exits, restricted
/// room prose, guarded NPC `def.lore`, and private NPC prompt fields.
#[must_use]
pub fn project_pack_for_client(raw_json: &str) -> Option<String> {
    let doc: Value = serde_json::from_str(raw_json).ok()?;
    project_pack_value_for_client(doc)
}

#[derive(Clone)]
struct PackCollector {
    entries: Vec<ClientPackEntry>,
    pack_rows: Vec<Value>,
    by_id: BTreeMap<String, usize>,
}

impl PackCollector {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            pack_rows: Vec::new(),
            by_id: BTreeMap::new(),
        }
    }

    fn finish(self) -> ClientContentBundle {
        ClientContentBundle {
            manifest_packs_value: Value::Array(self.pack_rows),
            entries: self.entries,
            by_id: self.by_id,
        }
    }

    fn push_projected_string(
        &mut self,
        id: &str,
        category: &'static str,
        projected: String,
        aliases: &[&str],
    ) {
        if self.by_id.contains_key(id) {
            return;
        }
        let sha = sha256_hex(projected.as_bytes());
        let idx = self.entries.len();
        self.pack_rows.push(json!({
            "id": id,
            "category": category,
            "sha256": sha,
            "bytes": projected.len(),
        }));
        self.entries.push(ClientPackEntry {
            id: Arc::from(id),
            category,
            sha256: Arc::from(sha),
            json: Arc::from(projected),
        });
        self.by_id.insert(id.to_owned(), idx);
        for &alias in aliases {
            if !alias.is_empty() {
                self.by_id.entry(alias.to_owned()).or_insert(idx);
            }
        }
    }

    fn push_projected(
        &mut self,
        id: &str,
        category: &'static str,
        raw_json: &str,
        aliases: &[&str],
    ) {
        if self.by_id.contains_key(id) {
            return;
        }
        let Some(projected) = project_pack_for_client(raw_json) else {
            return;
        };
        self.push_projected_string(id, category, projected, aliases);
    }

    fn push_projected_value(
        &mut self,
        id: &str,
        category: &'static str,
        doc: Value,
        aliases: &[&str],
    ) {
        if self.by_id.contains_key(id) {
            return;
        }
        let Some(projected) = project_pack_value_for_client(doc) else {
            return;
        };
        self.push_projected_string(id, category, projected, aliases);
    }

    fn push_manifest_neighborhood_if_applicable(&mut self, raw_json: &str) {
        let Ok(doc) = serde_json::from_str::<Value>(raw_json) else {
            return;
        };
        if !neighborhood_is_player_accessible(&doc) {
            return;
        }
        let Some(id) = doc
            .get("manifest")
            .and_then(Value::as_object)
            .filter(|m| m.get("kind").and_then(Value::as_str) == Some("neighborhood"))
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return;
        };
        let short = id.strip_prefix("neighborhood.").unwrap_or("");
        self.push_projected_value(&id, "neighborhood", doc, &[short]);
    }

    fn push_fielded_value_if_client_relevant(&mut self, doc: Value) {
        let manifest_id = doc
            .get("manifest")
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let (first_unit, first_weapon, first_arena) = {
            let Some(entries) = doc.get("entries").and_then(Value::as_object) else {
                return;
            };
            let mut u: Option<String> = None;
            let mut wp: Option<String> = None;
            let mut ar: Option<String> = None;
            for (entry_id, entry) in entries {
                match entry.get("kind").and_then(Value::as_str) {
                    Some("unit") if u.is_none() => u = Some(entry_id.clone()),
                    Some("weapon") if wp.is_none() => wp = Some(entry_id.clone()),
                    Some("arena") if ar.is_none() => ar = Some(entry_id.clone()),
                    _ => {}
                }
            }
            (u, wp, ar)
        };
        if let Some(unit_id) = first_unit {
            let slug = unit_id
                .strip_prefix("tacsim.unit.")
                .unwrap_or(&unit_id)
                .to_owned();
            self.push_projected_value(&slug, "unit", doc, &[&unit_id, &manifest_id]);
        } else if let Some(weapon_id) = first_weapon {
            let slug = weapon_id
                .strip_prefix("tacsim.weapon.")
                .unwrap_or(&weapon_id)
                .to_owned();
            self.push_projected_value(&slug, "weapon", doc, &[&weapon_id, &manifest_id]);
        } else if let Some(arena_id) = first_arena {
            let slug = arena_id.strip_prefix("tacsim.arena.").unwrap_or(&arena_id);
            let client_id = format!("arena.{slug}");
            self.push_projected_value(&client_id, "arena", doc, &[&arena_id, &manifest_id]);
        }
    }

    fn push_fielded_pack_if_client_relevant(&mut self, raw_json: &str) {
        let Ok(doc) = serde_json::from_str::<Value>(raw_json) else {
            return;
        };
        self.push_fielded_value_if_client_relevant(doc);
    }
}

/// Reconcile a serialized neighborhood document with the live world's applied
/// [`mush_atlas::Records`] (`RoomGeo` footprints/floors/names and `StructureRec`
/// floors), so worlds running an older applied revision or live geometry edits
/// receive exact matching room bounds and room sets.
fn reconcile_neighborhood_with_live_records(
    w: &World,
    records: &mush_atlas::Records,
    pack_id: &mush_core::CanonicalId,
    doc: &mut Value,
) {
    let live_rooms: BTreeMap<String, &mush_atlas::RoomGeo> = records
        .roomgeo
        .values()
        .filter(|geo| &geo.key.pack == pack_id)
        .map(|geo| (geo.key.entry.as_str().to_owned(), geo))
        .collect();
    if live_rooms.is_empty() {
        return;
    }

    let Some(entries) = doc
        .as_object_mut()
        .and_then(|root| root.get_mut("entries"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };

    // Drop room entries that are not part of the world's currently applied revision.
    let unapplied_rooms: Vec<String> = entries
        .iter()
        .filter_map(|(k, v)| {
            let is_room = v.get("kind").and_then(Value::as_str) == Some("room");
            (is_room && !live_rooms.contains_key(k)).then_some(k.clone())
        })
        .collect();
    for k in unapplied_rooms {
        entries.remove(&k);
    }

    // Overlay live RoomGeo footprint (`def.footprint.rect`), surface (`def.surface.floor`),
    // room kind, access, and live room object name using the exact `mush-atlas` pack wire schema
    // consumed by Unity's `TacSimContentLoader.ParseNeighborhoodJson`.
    for (entry_key, geo) in &live_rooms {
        let row = entries
            .entry(entry_key.clone())
            .or_insert_with(|| json!({ "kind": "room", "def": {} }));
        if let Some(def) = row.get_mut("def").and_then(Value::as_object_mut) {
            if let Some(obj) = w.get(geo.room) {
                def.insert("name".to_owned(), Value::String(obj.name.to_string()));
            }
            def.insert(
                "kind".to_owned(),
                json!(match geo.kind {
                    mush_atlas::RoomKind::Outdoor => "outdoor",
                    mush_atlas::RoomKind::Indoor => "indoor",
                    mush_atlas::RoomKind::Circulation => "circulation",
                }),
            );
            if let Some(site_key) = &geo.site {
                def.insert("site".to_owned(), json!(site_key.entry.as_str()));
            }
            match &geo.footprint {
                mush_atlas::Footprint::Rect(mush_atlas::Rect2(
                    mush_atlas::Point2(x0, y0),
                    mush_atlas::Point2(x1, y1),
                )) => {
                    def.insert(
                        "footprint".to_owned(),
                        json!({ "rect": [[*x0, *y0], [*x1, *y1]] }),
                    );
                }
                mush_atlas::Footprint::Polygon(mush_atlas::Polygon2(pts)) if !pts.is_empty() => {
                    let mut min_x = f64::INFINITY;
                    let mut min_y = f64::INFINITY;
                    let mut max_x = f64::NEG_INFINITY;
                    let mut max_y = f64::NEG_INFINITY;
                    for mush_atlas::Point2(px, py) in pts {
                        min_x = min_x.min(*px);
                        min_y = min_y.min(*py);
                        max_x = max_x.max(*px);
                        max_y = max_y.max(*py);
                    }
                    def.insert(
                        "footprint".to_owned(),
                        json!({ "rect": [[min_x, min_y], [max_x, max_y]] }),
                    );
                }
                mush_atlas::Footprint::Anchor { point, radius_m } => {
                    let r = radius_m.max(2.0);
                    def.insert(
                        "footprint".to_owned(),
                        json!({ "rect": [[point.0 - r, point.1 - r], [point.0 + r, point.1 + r]] }),
                    );
                }
                mush_atlas::Footprint::Polygon(_) => {}
            }
            match &geo.surface {
                mush_atlas::SurfaceRef::Ground => {
                    def.insert("surface".to_owned(), json!("ground"));
                }
                mush_atlas::SurfaceRef::Floor {
                    structure, index, ..
                } => {
                    def.insert(
                        "surface".to_owned(),
                        json!({ "floor": [structure.entry.as_str(), *index] }),
                    );
                }
            }
            def.insert(
                "access".to_owned(),
                json!(match geo.access {
                    mush_atlas::Access::Public => "public",
                    mush_atlas::Access::Restricted => "restricted",
                }),
            );
        }
    }

    // Overlay live NeighborhoodRec display name, region, and setting when present.
    for (nkey, nb) in &records.neighborhoods {
        if &nkey.pack != pack_id {
            continue;
        }
        let row = entries
            .entry(nkey.entry.as_str().to_owned())
            .or_insert_with(|| json!({ "kind": "neighborhood", "def": {} }));
        if let Some(def) = row.get_mut("def").and_then(Value::as_object_mut) {
            def.insert("name".to_owned(), Value::String(nb.name.clone()));
            def.insert(
                "region".to_owned(),
                Value::String(nb.region.pack.as_str().to_owned()),
            );
            def.insert("setting".to_owned(), Value::String(nb.setting.clone()));
        }
    }

    // Overlay live StructureRec floor definitions when present.
    for (skey, st) in &records.structures {
        if &skey.pack != pack_id {
            continue;
        }
        let row = entries
            .entry(skey.entry.as_str().to_owned())
            .or_insert_with(|| json!({ "kind": "structure", "def": {} }));
        if let Some(def) = row.get_mut("def").and_then(Value::as_object_mut)
            && let Ok(floors_val) = serde_json::to_value(&st.floors)
        {
            def.insert("floors".to_owned(), floors_val);
        }
    }
}

/// Build the player-safe client content bundle for the live `w` world and `actor`,
/// honoring `ContentState` applied/retired latches, `AtlasState` operator packs
/// and live `Records` geometry, `TacsimStore` applied revisions, and structural
/// neighborhood accessibility. Falls back to [`client_content_bundle`] only on a
/// bare detached `World` with neither `AtlasState` nor `TacsimStore` composed.
#[must_use]
pub fn world_content_bundle(w: &World, actor: ObjId) -> ClientContentBundle {
    let atlas = w.module_state::<mush_atlas::AtlasState>();
    let store = w.module_state::<TacsimStore>();
    if atlas.is_none() && store.is_none() {
        let base = client_content_bundle();
        return ClientContentBundle {
            manifest_packs_value: base.manifest_packs_value.clone(),
            entries: base.entries.clone(),
            by_id: base.by_id.clone(),
        };
    }

    let is_staff_viewer = mush_atlas::is_staff(w, actor);
    let content = w.module_state::<mush_content::ContentState>();
    let mut col = PackCollector::new();

    // 1. Live world neighborhoods & cast packs from `AtlasState` (`shelf` + `records`)
    if let Some(atlas) = atlas {
        for (id, pack) in &atlas.shelf {
            if let Some(content) = content {
                let Some(applied) = content.latches.applied(id) else {
                    continue;
                };
                if applied.is_retired() {
                    continue;
                }
            }
            if let Some(arena_slug) = id
                .as_str()
                .strip_prefix("tacsim.")
                .filter(|s| matches!(*s, "proving_ground" | "mars_canyon" | "winter_valley"))
            {
                if let Ok(doc) = serde_json::to_value(pack) {
                    let client_id = format!("arena_atlas.{arena_slug}");
                    col.push_projected_value(&client_id, "arena_atlas", doc, &[id.as_str()]);
                }
                continue;
            }
            if pack.manifest().kind() == mush_content::PackKind::Neighborhood {
                if let Ok(mut doc) = serde_json::to_value(pack) {
                    reconcile_neighborhood_with_live_records(w, &atlas.records, id, &mut doc);
                    if !is_staff_viewer && !neighborhood_is_player_accessible(&doc) {
                        continue;
                    }
                    let short = id.as_str().strip_prefix("neighborhood.").unwrap_or("");
                    col.push_projected_value(id.as_str(), "neighborhood", doc, &[short]);
                }
            } else if let Some(cast_slug) = id.as_str().strip_prefix("tacsim.cast.")
                && let Ok(doc) = serde_json::to_value(pack)
            {
                let client_id = format!("cast.{cast_slug}");
                col.push_projected_value(&client_id, "cast", doc, &[id.as_str()]);
            }
        }

        // Synthesize any applied neighborhood present in `atlas.records` whose source
        // pack is no longer on `atlas.shelf`.
        let mut record_packs = std::collections::BTreeSet::new();
        for nkey in atlas.records.neighborhoods.keys() {
            record_packs.insert(nkey.pack.clone());
        }
        for pack_id in record_packs {
            if col.by_id.contains_key(pack_id.as_str()) {
                continue;
            }
            if content.is_some_and(|c| {
                c.latches
                    .applied(&pack_id)
                    .is_some_and(mush_content::Applied::is_retired)
            }) {
                continue;
            }
            let version = content
                .and_then(|c| c.latches.applied(&pack_id))
                .map_or(1, mush_content::Applied::version);
            let mut doc = json!({
                "manifest": {
                    "id": pack_id.as_str(),
                    "version": version,
                    "kind": "neighborhood",
                    "projected": true,
                },
                "entries": {},
            });
            reconcile_neighborhood_with_live_records(w, &atlas.records, &pack_id, &mut doc);
            if !is_staff_viewer && !neighborhood_is_player_accessible(&doc) {
                continue;
            }
            let short = pack_id.as_str().strip_prefix("neighborhood.").unwrap_or("");
            col.push_projected_value(pack_id.as_str(), "neighborhood", doc, &[short]);
        }
    }

    // 2. Station public cast fallback when cast packs were seeded outside `atlas.shelf`
    for &(pack_id, json_src) in &CAST_SOURCES {
        let Ok(cid) = mush_core::CanonicalId::new(pack_id) else {
            continue;
        };
        if let Some(content) = content {
            match content.latches.applied(&cid) {
                Some(a) if a.is_retired() => continue,
                None if !content.latches.all().count().eq(&0) => {
                    // Include only if its companion neighborhood is applied or no cast latch exists
                    let slug = pack_id.strip_prefix("tacsim.cast.").unwrap_or(pack_id);
                    let nb_id = format!("neighborhood.{slug}");
                    if !col.by_id.contains_key(&nb_id) {
                        continue;
                    }
                }
                _ => {}
            }
        }
        let slug = pack_id.strip_prefix("tacsim.cast.").unwrap_or(pack_id);
        let client_id = format!("cast.{slug}");
        col.push_projected(&client_id, "cast", json_src, &[pack_id]);
    }

    // 3. Authoritative `GoodsTable` (standalone typed document outside `TacsimStore::packs()`)
    if let Ok(table) = crate::definitions::goods::goods_table()
        && let Ok(goods_doc) = serde_json::to_value(table)
    {
        col.push_projected_value("goods", "market", goods_doc, &["tacsim.goods"]);
    } else {
        col.push_projected("goods", "market", GOODS_JSON, &["tacsim.goods"]);
    }

    // 4. Live `TacsimStore` rules, arena definitions, & campaign-seed packs at world-applied versions
    if let Some(store) = store {
        let mut unique_ids = std::collections::BTreeSet::new();
        for loaded in store.packs() {
            unique_ids.insert(loaded.pack.manifest().id().clone());
        }
        for id in unique_ids {
            if content.is_some_and(|c| {
                c.latches
                    .applied(&id)
                    .is_some_and(mush_content::Applied::is_retired)
            }) {
                continue;
            }
            let active_ver = store.applied_in(w, &id).or_else(|| {
                store.newest_loaded(&id).filter(|&v| {
                    store.pack(&id, v).is_some_and(|lp| {
                        lp.pack.manifest().kind() != mush_content::PackKind::CampaignSeed
                            || lp
                                .pack
                                .entries()
                                .values()
                                .any(|e| matches!(e, crate::pack::PackEntry::Arena(_)))
                    })
                })
            });
            let Some(ver) = active_ver else {
                continue;
            };
            let Some(loaded) = store.pack(&id, ver) else {
                continue;
            };
            let Ok(doc) = loaded.pack.canonical_value() else {
                continue;
            };
            let has_fielded_def = doc
                .get("entries")
                .and_then(Value::as_object)
                .is_some_and(|e| {
                    e.values().any(|row| {
                        matches!(
                            row.get("kind").and_then(Value::as_str),
                            Some("unit" | "weapon" | "arena")
                        )
                    })
                });
            if has_fielded_def {
                col.push_fielded_value_if_client_relevant(doc);
            } else if loaded.pack.manifest().kind() == mush_content::PackKind::CampaignSeed {
                let has_location = doc
                    .get("entries")
                    .and_then(Value::as_object)
                    .is_some_and(|e| {
                        e.values()
                            .any(|row| row.get("kind").and_then(Value::as_str) == Some("location"))
                    });
                if has_location {
                    let slug = id
                        .as_str()
                        .strip_prefix("tacsim.lane.")
                        .unwrap_or(id.as_str());
                    col.push_projected_value(slug, "lane", doc, &[id.as_str()]);
                }
            }
        }

        // Fill in stock `Catalog` unit/weapon/arena definitions (`fielded_sources` + conduct carrier)
        // that were compiled into `store.definitions()` rather than repeated in `TacsimConfig.packs`,
        // skipping any pack overridden above or retired in `ContentState`.
        for raw_src in fielded_sources()
            .into_iter()
            .map(|s| s.bytes)
            .chain(std::iter::once(CONDUCT_CARRIER_JSON.to_owned()))
        {
            let Ok(doc) = serde_json::from_str::<Value>(&raw_src) else {
                continue;
            };
            let manifest_id = doc
                .get("manifest")
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !manifest_id.is_empty() {
                if col.by_id.contains_key(manifest_id) {
                    continue;
                }
                if let Ok(cid) = mush_core::CanonicalId::new(manifest_id)
                    && content.is_some_and(|c| {
                        c.latches
                            .applied(&cid)
                            .is_some_and(mush_content::Applied::is_retired)
                    })
                {
                    continue;
                }
            }
            col.push_fielded_value_if_client_relevant(doc);
        }

        // Companion ground heightfield atlases for active simulator arenas
        for (arena_key, atlas_key, raw_src, alias) in [
            (
                "arena.proving_ground",
                "arena_atlas.proving_ground",
                PROVING_GROUND_ATLAS_JSON,
                "tacsim.proving_ground",
            ),
            (
                "arena.mars_canyon",
                "arena_atlas.mars_canyon",
                MARS_CANYON_ATLAS_JSON,
                "tacsim.mars_canyon",
            ),
            (
                "arena.winter_valley",
                "arena_atlas.winter_valley",
                WINTER_VALLEY_ATLAS_JSON,
                "tacsim.winter_valley",
            ),
        ] {
            if col.by_id.contains_key(arena_key) && !col.by_id.contains_key(atlas_key) {
                col.push_projected(atlas_key, "arena_atlas", raw_src, &[alias]);
            }
        }
    }

    col.finish()
}

/// Lazily computed static bundle of projected client content packs and SHA-256 digests.
#[must_use]
pub fn client_content_bundle() -> &'static ClientContentBundle {
    static BUNDLE: OnceLock<ClientContentBundle> = OnceLock::new();
    BUNDLE.get_or_init(|| {
        let mut col = PackCollector::new();

        // 1. Starter OOC & Staff Lobby neighborhoods (`mush-atlas`)
        for &(_id, json_src) in &mush_atlas::starter::SOURCES {
            col.push_manifest_neighborhood_if_applicable(json_src);
        }

        // 2. All station, surface site, wide site, and relay neighborhoods (`tacsim`)
        for &(_id, json_src) in STATION_SOURCES
            .iter()
            .chain(SITE_SOURCES.iter())
            .chain(WIDE_SITE_SOURCES.iter())
        {
            col.push_manifest_neighborhood_if_applicable(json_src);
        }
        for json_src in crate::relay_content::GEOGRAPHY {
            col.push_manifest_neighborhood_if_applicable(json_src);
        }

        // 3. Station public cast metadata (`cast.<slug>`, with lore & prompt internals stripped)
        for &(pack_id, json_src) in &CAST_SOURCES {
            let slug = pack_id.strip_prefix("tacsim.cast.").unwrap_or(pack_id);
            let client_id = format!("cast.{slug}");
            col.push_projected(&client_id, "cast", json_src, &[pack_id]);
        }

        // 4. Economy goods table & trade corridor lane
        col.push_projected("goods", "market", GOODS_JSON, &["tacsim.goods"]);
        col.push_projected(
            "ceres_phobos",
            "lane",
            LANE_JSON,
            &["tacsim.lane.ceres_phobos"],
        );

        // 5. All fielded units, weapons, and arena definitions (`fielded_sources` + conduct carrier)
        for src in fielded_sources() {
            col.push_fielded_pack_if_client_relevant(&src.bytes);
        }
        col.push_fielded_pack_if_client_relevant(CONDUCT_CARRIER_JSON);

        // 6. Simulator arena ground atlases
        col.push_projected(
            "arena_atlas.proving_ground",
            "arena_atlas",
            PROVING_GROUND_ATLAS_JSON,
            &["tacsim.proving_ground"],
        );
        col.push_projected(
            "arena_atlas.mars_canyon",
            "arena_atlas",
            MARS_CANYON_ATLAS_JSON,
            &["tacsim.mars_canyon"],
        );
        col.push_projected(
            "arena_atlas.winter_valley",
            "arena_atlas",
            WINTER_VALLEY_ATLAS_JSON,
            &["tacsim.winter_valley"],
        );

        ClientContentBundle {
            manifest_packs_value: Value::Array(col.pack_rows),
            entries: col.entries,
            by_id: col.by_id,
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorldContentFingerprint(u64, u64);

fn hash_str_dual(h1: &mut std::collections::hash_map::DefaultHasher, h2: &mut u64, s: &str) {
    use std::hash::Hash as _;
    s.hash(h1);
    hash_fnv1a64(h2, s.as_bytes());
    hash_fnv1a64(h2, &[0xff]);
}

fn hash_bytes_dual(h1: &mut std::collections::hash_map::DefaultHasher, h2: &mut u64, bytes: &[u8]) {
    use std::hash::Hash as _;
    bytes.hash(h1);
    hash_fnv1a64(h2, bytes);
}

fn hash_fnv1a64(state: &mut u64, bytes: &[u8]) {
    let mut h = *state;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    *state = h;
}

fn hash_footprint_dual(
    h1: &mut std::collections::hash_map::DefaultHasher,
    h2: &mut u64,
    footprint: &mush_atlas::Footprint,
) {
    match footprint {
        mush_atlas::Footprint::Rect(rect) => {
            hash_bytes_dual(h1, h2, &[1]);
            hash_bytes_dual(h1, h2, &rect.0.0.to_bits().to_le_bytes());
            hash_bytes_dual(h1, h2, &rect.0.1.to_bits().to_le_bytes());
            hash_bytes_dual(h1, h2, &rect.1.0.to_bits().to_le_bytes());
            hash_bytes_dual(h1, h2, &rect.1.1.to_bits().to_le_bytes());
        }
        mush_atlas::Footprint::Polygon(poly) => {
            hash_bytes_dual(h1, h2, &[2]);
            hash_bytes_dual(h1, h2, &(poly.0.len() as u64).to_le_bytes());
            for pt in &poly.0 {
                hash_bytes_dual(h1, h2, &pt.0.to_bits().to_le_bytes());
                hash_bytes_dual(h1, h2, &pt.1.to_bits().to_le_bytes());
            }
        }
        mush_atlas::Footprint::Anchor { point, radius_m } => {
            hash_bytes_dual(h1, h2, &[3]);
            hash_bytes_dual(h1, h2, &point.0.to_bits().to_le_bytes());
            hash_bytes_dual(h1, h2, &point.1.to_bits().to_le_bytes());
            hash_bytes_dual(h1, h2, &radius_m.to_bits().to_le_bytes());
        }
    }
}

fn hash_surface_dual(
    h1: &mut std::collections::hash_map::DefaultHasher,
    h2: &mut u64,
    surface: &mush_atlas::SurfaceRef,
) {
    match surface {
        mush_atlas::SurfaceRef::Ground => hash_bytes_dual(h1, h2, &[0]),
        mush_atlas::SurfaceRef::Floor { structure, index } => {
            hash_bytes_dual(h1, h2, &[1]);
            hash_str_dual(h1, h2, structure.pack.as_str());
            hash_str_dual(h1, h2, structure.entry.as_str());
            hash_bytes_dual(h1, h2, &index.to_le_bytes());
        }
    }
}

fn hash_atlas_state(
    h1: &mut std::collections::hash_map::DefaultHasher,
    h2: &mut u64,
    w: &World,
    atlas: &mush_atlas::AtlasState,
) {
    hash_bytes_dual(h1, h2, b"|shelf:");
    for (id, pack) in &atlas.shelf {
        hash_str_dual(h1, h2, id.as_str());
        hash_bytes_dual(h1, h2, &pack.manifest().version().to_le_bytes());
        hash_bytes_dual(h1, h2, pack.manifest().digest().as_bytes());
    }
    hash_bytes_dual(h1, h2, b"|nb:");
    for (nkey, nb) in &atlas.records.neighborhoods {
        hash_str_dual(h1, h2, nkey.pack.as_str());
        hash_str_dual(h1, h2, nkey.entry.as_str());
        hash_bytes_dual(h1, h2, &nb.revision.to_le_bytes());
        hash_str_dual(h1, h2, &nb.name);
        hash_str_dual(h1, h2, &nb.setting);
    }
    hash_bytes_dual(h1, h2, b"|st:");
    for (skey, st) in &atlas.records.structures {
        hash_str_dual(h1, h2, skey.pack.as_str());
        hash_str_dual(h1, h2, skey.entry.as_str());
        hash_bytes_dual(h1, h2, &st.revision.to_le_bytes());
        hash_bytes_dual(h1, h2, &(st.floors.len() as u64).to_le_bytes());
    }
    hash_bytes_dual(h1, h2, b"|geo:");
    for (room_id, geo) in &atlas.records.roomgeo {
        hash_bytes_dual(h1, h2, &room_id.0.to_le_bytes());
        hash_bytes_dual(h1, h2, &geo.revision.to_le_bytes());
        hash_str_dual(h1, h2, geo.key.pack.as_str());
        hash_str_dual(h1, h2, geo.key.entry.as_str());
        if let Some(obj) = w.get(*room_id) {
            hash_str_dual(h1, h2, &obj.name);
        }
        hash_surface_dual(h1, h2, &geo.surface);
        hash_footprint_dual(h1, h2, &geo.footprint);
    }
}

/// Fast, non-allocating revision/geometry fingerprint for [`ClientContentQuery`] cache hits.
///
/// Avoids per-query `serde_json::to_vec(&geo.footprint)` heap serialization and SHA-256 hashing
/// while still invalidating whenever applied latches, atlas packs, room geometry/names, or
/// TacSim pack versions change.
fn world_content_fingerprint(w: &World, actor: ObjId) -> WorldContentFingerprint {
    use std::hash::Hasher as _;

    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    let mut h2: u64 = 0xcbf2_9ce4_8422_2325;
    let is_staff_viewer = mush_atlas::is_staff(w, actor);
    hash_bytes_dual(&mut h1, &mut h2, &[u8::from(is_staff_viewer)]);
    if let Some(content) = w.module_state::<mush_content::ContentState>() {
        hash_bytes_dual(&mut h1, &mut h2, b"|content:");
        for applied in content.latches.all() {
            hash_str_dual(&mut h1, &mut h2, applied.pack().as_str());
            hash_bytes_dual(&mut h1, &mut h2, &applied.version().to_le_bytes());
            hash_bytes_dual(&mut h1, &mut h2, applied.digest().as_bytes());
            hash_bytes_dual(&mut h1, &mut h2, &[u8::from(applied.is_retired())]);
        }
    }
    if let Some(atlas) = w.module_state::<mush_atlas::AtlasState>() {
        hash_atlas_state(&mut h1, &mut h2, w, atlas);
    }
    if let Some(store) = w.module_state::<TacsimStore>() {
        hash_bytes_dual(&mut h1, &mut h2, b"|tacsim:");
        hash_bytes_dual(&mut h1, &mut h2, &[u8::from(store.enabled())]);
        for loaded in store.packs() {
            let id = loaded.pack.manifest().id();
            hash_str_dual(&mut h1, &mut h2, id.as_str());
            hash_bytes_dual(
                &mut h1,
                &mut h2,
                &loaded.pack.manifest().version().to_le_bytes(),
            );
            let applied = store.applied_in(w, id).unwrap_or(0);
            hash_bytes_dual(&mut h1, &mut h2, &applied.to_le_bytes());
        }
    }
    WorldContentFingerprint(h1.finish(), h2)
}

/// Register the `port.trade` and `tacsim.content` pulled panel feeds and query handlers.
///
/// # Errors
///
/// [`ModError`] if either feed or query handler name is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    drop(reg.panel_feed(PORT_TRADE_FEED)?);
    reg.query_handler(PORT_TRADE_FEED, Arc::new(PortTradeQuery))?;
    drop(reg.panel_feed(CLIENT_CONTENT_FEED)?);
    reg.query_handler(CLIENT_CONTENT_FEED, Arc::new(ClientContentQuery::new()))?;
    Ok(())
}

struct ClientContentQuery {
    player_cache: Mutex<Option<(WorldContentFingerprint, Arc<ClientContentBundle>)>>,
    staff_cache: Mutex<Option<(WorldContentFingerprint, Arc<ClientContentBundle>)>>,
}

impl ClientContentQuery {
    fn new() -> Self {
        Self {
            player_cache: Mutex::new(None),
            staff_cache: Mutex::new(None),
        }
    }

    fn bundle_for(&self, w: &World, actor: ObjId) -> Arc<ClientContentBundle> {
        let is_staff = mush_atlas::is_staff(w, actor);
        let slot = if is_staff {
            &self.staff_cache
        } else {
            &self.player_cache
        };
        let fp = world_content_fingerprint(w, actor);
        if let Ok(guard) = slot.lock()
            && let Some((cached_fp, cached_bundle)) = guard.as_ref()
            && *cached_fp == fp
        {
            return Arc::clone(cached_bundle);
        }
        let built = Arc::new(world_content_bundle(w, actor));
        if let Ok(mut guard) = slot.lock() {
            *guard = Some((fp, Arc::clone(&built)));
        }
        built
    }
}

impl QueryHandler for ClientContentQuery {
    fn query(&self, w: &World, actor: ObjId, params: &Value) -> Result<OutputEvent, RichText> {
        let op = params
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("manifest");
        if w.module_state::<TacsimStore>()
            .is_some_and(|store| !store.enabled())
        {
            let data = if op == "pack" {
                let req_id = params.get("id").and_then(Value::as_str).unwrap_or("");
                json!({
                    "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                    "enabled": false,
                    "op": "pack",
                    "found": false,
                    "id": req_id,
                })
            } else {
                json!({
                    "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                    "enabled": false,
                    "op": "manifest",
                    "packs": [],
                })
            };
            return Ok(OutputEvent::Panel {
                feed: SmolStr::new_static(CLIENT_CONTENT_FEED),
                kind: PanelFrameKind::Keyframe,
                data,
            });
        }
        let bundle = self.bundle_for(w, actor);
        let data = if op == "pack" {
            let req_id = params.get("id").and_then(Value::as_str).unwrap_or("");
            match bundle.find(req_id) {
                Some(entry) => json!({
                    "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                    "op": "pack",
                    "found": true,
                    "id": entry.id.as_ref(),
                    "requested_id": req_id,
                    "category": entry.category,
                    "sha256": entry.sha256.as_ref(),
                    "json": entry.json.as_ref(),
                }),
                None => json!({
                    "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                    "op": "pack",
                    "found": false,
                    "id": req_id,
                }),
            }
        } else {
            json!({
                "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                "op": "manifest",
                "packs": bundle.manifest_packs_value.clone(),
            })
        };
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(CLIENT_CONTENT_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

struct PortTradeQuery;

fn actor_credit_balance(w: &World, actor: ObjId) -> (i64, String) {
    let Some(economy) = w.module_state::<EconomyStore>() else {
        return (0, "0.00cr".to_owned());
    };
    let Some(instrument) = InstrumentId::new(CREDIT) else {
        return (0, "0.00cr".to_owned());
    };
    let available = own_account(w, actor)
        .and_then(mush_economy::Account::available)
        .unwrap_or(Amount::ZERO);
    let available_minor = available.minor().max(0);
    let text = mush_economy::render_amount(
        economy.snapshot(),
        &instrument,
        Amount::from_minor(available_minor),
    );
    (available_minor, text)
}

/// Resolve the market venue where `actor` can actually execute `Presence::Here`
/// trades (`PortVenueGate` excludes hull rooms, so no hull-room fallback is used).
fn resolve_actor_venue(w: &World, actor: ObjId) -> (Option<mush_market::MarketId>, Option<String>) {
    let Some(loc) = w.get(actor).and_then(|o| o.location) else {
        return (None, None);
    };
    use mush_market::VenueGate as _;
    if let Some(v) = PortVenueGate.venue_of_room(w, loc) {
        return (Some(v), None);
    }
    let docked_port_hint =
        hull_of_room(w, loc).and_then(|(view, _)| match berth_of(w, view.hull()) {
            Berth::Docked { def, .. } if !def.venue.as_str().is_empty() => Some(port_name(def)),
            _ => None,
        });
    (None, docked_port_hint)
}

fn format_price(w: &World, instrument: Option<&InstrumentId>, amount: Amount) -> String {
    if let (Some(economy), Some(inst)) = (w.module_state::<EconomyStore>(), instrument) {
        mush_economy::render_amount(economy.snapshot(), inst, amount)
    } else {
        money(w, amount)
    }
}

fn build_open_quotes_json(w: &World, market_store: &MarketStore, actor: ObjId) -> Vec<Value> {
    let buyer = PrincipalId::Object(actor);
    let now = w.now();
    let mut out = Vec::new();
    for q in market_store.quotes_of_buyer(&buyer) {
        if q.contract.is_some() || q.settled.is_some() || now >= q.expires.at {
            continue;
        }
        let listing_still_matches = market_store
            .listing(q.listing)
            .is_some_and(|l| l.revision == q.observed.listing && l.quantity >= q.quantity);
        if !listing_still_matches {
            continue;
        }
        let total_minor = q
            .price
            .minor()
            .saturating_mul(i64::try_from(q.quantity).unwrap_or(i64::MAX));
        out.push(json!({
            "quote_id": q.id.0,
            "listing_id": q.listing.0,
            "good": q.catalog.code.as_str(),
            "market": q.market.as_str(),
            "quantity": q.quantity,
            "unit_price_minor": q.price.minor(),
            "total_minor": total_minor,
            "price_text": money(w, q.price),
            "total_text": money(w, Amount::from_minor(total_minor)),
            "expires_at": q.expires.at,
        }));
    }
    out
}

fn build_port_trade_frame(
    ctx: Option<&HookReadContext<'_>>,
    w: &World,
    actor: ObjId,
    params: &Value,
) -> Value {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return json!({
            "schema": { "id": PORT_TRADE_FEED, "version": SCHEMA_VERSION },
            "enabled": false,
        });
    };
    if !store.enabled() {
        return json!({
            "schema": { "id": PORT_TRADE_FEED, "version": SCHEMA_VERSION },
            "enabled": false,
        });
    }
    let (balance_minor, balance_text) = actor_credit_balance(w, actor);
    let (venue, aboard_docked_port) = resolve_actor_venue(w, actor);
    let venue_port_label = venue
        .as_ref()
        .and_then(|v| port_of_venue(w, v))
        .map(|(def, _)| port_name(def));

    let after_cursor = params.get("after").and_then(Value::as_u64).map(ListingId);

    let mut listings_json = Vec::new();
    let mut next_after: Option<u64> = None;
    let mut open_quotes_json = Vec::new();

    if let Some(market_store) = w.module_state::<MarketStore>() {
        open_quotes_json = build_open_quotes_json(w, market_store, actor);
        if let Some(v) = venue.as_ref() {
            let filter = MarketFilter::Market(v.clone());
            let page = mush_market::visible(w, actor, Some(&filter), MARKET_PAGE, after_cursor);
            next_after = page.next.map(|id| id.0);
            let goods = store.goods();
            for id in &page.ids {
                let Some(l) = market_store.listing(*id) else {
                    continue;
                };
                let summary = mush_market::summarize_listing(w, l);
                let is_bid = l.source.is_wanted();
                let side = if is_bid { "bid" } else { "ask" };
                let good_code = l.catalog.code.to_string();
                let good_label = goods
                    .and_then(|g| {
                        mush_core::CanonicalId::new(&good_code)
                            .ok()
                            .and_then(|cid| g.good(&cid))
                    })
                    .map_or_else(|| good_code.clone(), |row| row.label.clone());
                let seller_display = if summary.seller_name.is_empty() {
                    l.market.as_str().to_owned()
                } else {
                    summary.seller_name
                };
                listings_json.push(json!({
                    "id": l.id.0,
                    "side": side,
                    "good": good_code,
                    "good_label": good_label,
                    "vendor_name": seller_display,
                    "is_personal": l.is_personal(),
                    "funded": summary.funded,
                    "unit_price_minor": summary.price.minor(),
                    "price_text": format_price(w, summary.instrument.as_ref(), summary.price),
                    "available_qty": l.quantity,
                }));
            }
        }
    }

    let ship_json = helm_of(w, actor).and_then(|charter| {
        let view = hull_view(w, charter.hull)?;
        let (docked, port_entry) = match berth_of(w, charter.hull) {
            Berth::Docked { map, .. } => (true, Some(map.entry.to_string())),
            Berth::UnderWay | Berth::Elsewhere => (false, None),
        };
        let (standing_word, sales, buyout_minor) = match charter.standing {
            crate::port::coop::Standing::Fleet => ("fleet", 0_u32, 0_i64),
            crate::port::coop::Standing::Title => ("title", 0_u32, 0_i64),
            crate::port::coop::Standing::Charter(contract) => store
                .enrolment_for_lease(contract.0)
                .filter(|e| e.status == EnrolmentStatus::Active)
                .map_or(("charter", 0_u32, 0_i64), |e| {
                    ("charter", e.sales, e.buyout_minor)
                }),
        };
        let stowage = ctx.and_then(|c| hull_stowage_json(c, w, store, &view));
        Some(json!({
            "hull_id": charter.hull.to_string(),
            "hull_name": name_of(w, charter.hull),
            "mark": view.row.mark,
            "unit": view.unit.to_string(),
            "class": view.class_word(),
            "docked": docked,
            "port": port_entry,
            "standing": standing_word,
            "sales": sales,
            "buyout_minor": buyout_minor,
            "buyout_text": money(w, Amount::from_minor(buyout_minor)),
            "stowage": stowage,
        }))
    });

    let actor_room = w.get(actor).and_then(|o| o.location);
    let berth_json = actor_room.and_then(|room| berth_gallery_json(w, store, actor, room));
    let simulator_json = actor_room.and_then(|room| simulator_lobby_json(w, actor, room));

    json!({
        "schema": { "id": PORT_TRADE_FEED, "version": SCHEMA_VERSION },
        "enabled": true,
        "venue": venue.as_ref().map(mush_market::MarketId::as_str),
        "port_name": venue_port_label,
        "aboard_docked_port": aboard_docked_port,
        "balance_minor": balance_minor,
        "balance_text": balance_text,
        "listings": listings_json,
        "next_after": next_after,
        "open_quotes": open_quotes_json,
        "ship": ship_json,
        "berth": berth_json,
        "simulator": simulator_json,
    })
}

impl QueryHandler for PortTradeQuery {
    fn query(&self, w: &World, actor: ObjId, params: &Value) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(PORT_TRADE_FEED),
            kind: PanelFrameKind::Keyframe,
            data: build_port_trade_frame(None, w, actor, params),
        })
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &Value,
    ) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(PORT_TRADE_FEED),
            kind: PanelFrameKind::Keyframe,
            data: build_port_trade_frame(Some(context), context.world(), actor, params),
        })
    }
}

/// Verify that every projected pack in `bundle` strips secrets, lore, quests, hooks,
/// hidden exits, restricted-room prose, private NPC prompts, and lane vendor/reserve data.
/// Validates structurally that a projected [`ClientContentBundle`] contains only client-safe
/// entry kinds and fields across every included pack, without hardcoding campaign-specific pack IDs.
///
/// # Errors
///
/// Returns a descriptive failure string if any non-public entry kind or field remains.
#[doc(hidden)]
pub fn validate_projected_bundle_cleanliness(bundle: &ClientContentBundle) -> Result<(), String> {
    if bundle.entries.is_empty() {
        return Err("bundle has no entries".to_owned());
    }
    for entry in &bundle.entries {
        if entry.sha256.len() != 64 {
            return Err(format!("invalid sha256 on `{}`", entry.id));
        }
        let parsed: Value = serde_json::from_str(&entry.json)
            .map_err(|e| format!("invalid json on `{}`: {e}", entry.id))?;
        if parsed.get("brief").is_some() {
            return Err(format!("`brief` leaked on `{}`", entry.id));
        }
        if let Some(entries) = parsed.get("entries").and_then(Value::as_object) {
            for (row_id, row) in entries {
                let kind = row.get("kind").and_then(Value::as_str).unwrap_or("");
                if !is_allowed_client_entry_kind(kind) {
                    return Err(format!(
                        "disallowed entry kind `{kind}` (`{row_id}`) leaked on `{}`",
                        entry.id
                    ));
                }
                if let Some(def) = row.get("def").and_then(Value::as_object) {
                    if def.get("reveal").and_then(Value::as_str) == Some("staff") {
                        return Err(format!("staff reveal leaked on `{}:{row_id}`", entry.id));
                    }
                    if def.get("visibility").and_then(Value::as_str) == Some("hidden") {
                        return Err(format!("hidden entry leaked on `{}:{row_id}`", entry.id));
                    }
                    let portal_vis = def
                        .get("portal")
                        .and_then(Value::as_object)
                        .and_then(|p| p.get("visibility"))
                        .and_then(Value::as_str);
                    if portal_vis == Some("hidden") {
                        return Err(format!("hidden portal leaked on `{}:{row_id}`", entry.id));
                    }
                    for forbidden in [
                        "lore",
                        "lore_tags",
                        "knowledge",
                        "vendors",
                        "treasury",
                        "orgs",
                        "programme",
                        "kirkwood_picket",
                    ] {
                        if def.contains_key(forbidden) {
                            return Err(format!(
                                "forbidden key `{forbidden}` leaked on `{}:{row_id}`",
                                entry.id
                            ));
                        }
                    }
                    if kind == "room"
                        && def.get("access").and_then(Value::as_str) == Some("restricted")
                        && (def.contains_key("desc") || def.contains_key("details"))
                    {
                        return Err(format!(
                            "restricted room prose leaked on `{}:{row_id}`",
                            entry.id
                        ));
                    }
                    if let Some(prof) = def.get("profile").and_then(Value::as_object) {
                        for forbidden_prof in ["persona", "knows", "does_not_know", "fallback"] {
                            if prof.contains_key(forbidden_prof) {
                                return Err(format!(
                                    "private profile `{forbidden_prof}` leaked on `{}:{row_id}`",
                                    entry.id
                                ));
                            }
                        }
                    }
                }
            }
        }
        for raw_forbidden in [
            "\"secrecy\":\"guarded\"",
            "\"secrecy\":\"secret\"",
            "\"gross_t\":",
            "\"ask_minor\":",
            "\"bid_minor\":",
            "\"intake_cap\":",
            "\"vendors\":",
            "\"kirkwood_picket\":",
        ] {
            if entry.json.contains(raw_forbidden) {
                return Err(format!(
                    "raw forbidden substring `{raw_forbidden}` leaked on `{}`",
                    entry.id
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn client_content_bundle_strips_secrets_hidden_exits_and_prompt_internals() -> TestResult {
        let bundle = client_content_bundle();
        if let Err(reason) = validate_projected_bundle_cleanliness(bundle) {
            return crate::test_checks::failed(&reason);
        }
        check!(bundle.find("neighborhood.staff_lobby").is_none());
        check!(bundle.find("neighborhood.ooc_area").is_some());
        let Some(ceres) = bundle.find("neighborhood.ceres_freeport") else {
            return crate::test_checks::failed("missing ceres_freeport pack");
        };
        for hidden in ["secret.slowdown", "exit.c14_c15", "exit.c05_c16"] {
            check!(!ceres.json.contains(hidden));
        }
        let Some(lane) = bundle.find("ceres_phobos") else {
            return crate::test_checks::failed("missing ceres_phobos lane pack");
        };
        for lane_secret in [
            "\"vendors\"",
            "\"deposit\"",
            "\"crew_offer\"",
            "\"passenger\"",
            "\"facility\"",
            "\"kirkwood_picket\"",
            "\"gross_t\"",
        ] {
            check!(!lane.json.contains(lane_secret));
        }
        Ok(())
    }

    #[test]
    fn client_content_bundle_covers_every_content_neighborhood_and_unit_file() -> TestResult {
        let bundle = client_content_bundle();
        let content_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content");

        for entry in std::fs::read_dir(content_root.join("atlas"))? {
            let entry = entry?;
            let fname = entry.file_name().to_string_lossy().into_owned();
            if let Some(pack_id) = fname
                .strip_prefix("neighborhood.")
                .and_then(|s| s.strip_suffix(".json"))
                .map(|slug| format!("neighborhood.{slug}"))
                && bundle.find(&pack_id).is_none()
            {
                return crate::test_checks::failed(&format!(
                    "neighborhood file `{fname}` missing from client_content_bundle"
                ));
            }
        }

        for entry in std::fs::read_dir(content_root.join("units"))? {
            let entry = entry?;
            let fname = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = fname.strip_suffix(".json")
                && bundle.find(stem).is_none()
            {
                return crate::test_checks::failed(&format!(
                    "unit file `{fname}` missing from client_content_bundle"
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn client_content_query_answers_manifest_aliases_and_isolates_staff_player_caches() -> TestResult
    {
        let mut tw = mush_testkit::TestWorld::new();
        let player = tw.spawn_player("PlayerViewer");
        let w = &tw.engine().world;
        let q = ClientContentQuery::new();

        let staff_b1 = q.bundle_for(w, mush_core::GOD);
        let player_b1 = q.bundle_for(w, player);
        let staff_b2 = q.bundle_for(w, mush_core::GOD);
        let player_b2 = q.bundle_for(w, player);
        check!(Arc::ptr_eq(&staff_b1, &staff_b2));
        check!(Arc::ptr_eq(&player_b1, &player_b2));

        let Ok(ev) = q.query(w, ObjId(1), &json!({"op": "manifest"})) else {
            return crate::test_checks::failed("manifest query failed");
        };
        let OutputEvent::Panel { feed, data, .. } = ev else {
            return crate::test_checks::failed("expected panel frame");
        };
        check_eq!(feed.as_str(), CLIENT_CONTENT_FEED);
        check_eq!(data["op"], json!("manifest"));
        check!(data["packs"].as_array().is_some_and(|a| a.len() >= 50));

        for alias in ["dray", "tacsim.unit.dray", "tacsim.pack.dray"] {
            let Ok(dray) = q.query(w, ObjId(1), &json!({"op": "pack", "id": alias})) else {
                return crate::test_checks::failed("pack query failed");
            };
            let OutputEvent::Panel { data: pdata, .. } = dray else {
                return crate::test_checks::failed("expected panel frame");
            };
            check_eq!(pdata["found"], json!(true));
            check_eq!(pdata["id"], json!("dray"));
        }
        Ok(())
    }

    #[test]
    fn reconcile_neighborhood_emits_client_footprint_and_surface_schema() -> TestResult {
        use mush_atlas::{
            Access, Footprint, Point2, RecordKey, Records, Rect2, RoomGeo, RoomKind, StructureRec,
            SurfaceRef,
        };
        use mush_core::CanonicalId;

        let mut tw = mush_testkit::TestWorld::new();
        let r1 =
            tw.world_mut()
                .create(mush_core::ObjKind::Room, "Live Moved Room", ObjId(1), None)?;
        let r2 =
            tw.world_mut()
                .create(mush_core::ObjKind::Room, "Synthesized Room", ObjId(1), None)?;

        let pack_id = CanonicalId::new("neighborhood.test_station")?;
        let nb_key = RecordKey::new(pack_id.clone(), CanonicalId::new("neighborhood.main")?);
        let st_key = RecordKey::new(pack_id.clone(), CanonicalId::new("structure.hub")?);

        let mut records = Records::default();
        records.structures.insert(
            st_key.clone(),
            StructureRec {
                key: st_key.clone(),
                site: RecordKey::new(pack_id.clone(), CanonicalId::new("site.hub")?),
                footprint: mush_atlas::Polygon2(vec![
                    Point2(0.0, 0.0),
                    Point2(200.0, 0.0),
                    Point2(200.0, 250.0),
                    Point2(0.0, 250.0),
                ]),
                floors: vec![
                    mush_atlas::record::Floor {
                        index: 1,
                        name: Some("Deck 1".to_owned()),
                        z_offset_m: 0.0,
                        height_m: 4.5,
                        tile: None,
                    },
                    mush_atlas::record::Floor {
                        index: 2,
                        name: Some("Deck 2".to_owned()),
                        z_offset_m: 5.0,
                        height_m: 4.5,
                        tile: None,
                    },
                    mush_atlas::record::Floor {
                        index: 3,
                        name: Some("Deck 3".to_owned()),
                        z_offset_m: 10.0,
                        height_m: 4.5,
                        tile: None,
                    },
                ],
                revision: 1,
            },
        );
        records.roomgeo.insert(
            r1,
            RoomGeo {
                room: r1,
                key: RecordKey::new(pack_id.clone(), CanonicalId::new("room.existing")?),
                neighborhood: nb_key.clone(),
                site: None,
                surface: SurfaceRef::Floor {
                    structure: st_key.clone(),
                    index: 3,
                },
                footprint: Footprint::Rect(Rect2(Point2(100.0, 200.0), Point2(120.0, 220.0))),
                kind: RoomKind::Indoor,
                access: Access::Public,
                revision: 2,
            },
        );
        records.roomgeo.insert(
            r2,
            RoomGeo {
                room: r2,
                key: RecordKey::new(pack_id.clone(), CanonicalId::new("room.synth")?),
                neighborhood: nb_key,
                site: None,
                surface: SurfaceRef::Floor {
                    structure: st_key,
                    index: 2,
                },
                footprint: Footprint::Rect(Rect2(Point2(40.0, 50.0), Point2(65.0, 80.0))),
                kind: RoomKind::Circulation,
                access: Access::Public,
                revision: 1,
            },
        );

        let mut doc = json!({
            "manifest": { "id": "neighborhood.test_station", "version": 1, "kind": "neighborhood" },
            "entries": {
                "room.existing": {
                    "kind": "room",
                    "def": {
                        "name": "Stale Shelf Room",
                        "footprint": { "rect": [[0.0, 0.0], [10.0, 10.0]] },
                        "surface": { "floor": ["structure.hub", 1] },
                    }
                },
                "room.unapplied_v2": {
                    "kind": "room",
                    "def": {
                        "name": "Future V2 Room",
                        "footprint": { "rect": [[300.0, 300.0], [320.0, 320.0]] },
                    }
                }
            }
        });

        reconcile_neighborhood_with_live_records(&tw.engine().world, &records, &pack_id, &mut doc);

        check!(doc["entries"].get("room.unapplied_v2").is_none());
        let ex_def = &doc["entries"]["room.existing"]["def"];
        check_eq!(ex_def["name"], json!("Live Moved Room"));
        check_eq!(
            ex_def["footprint"]["rect"],
            json!([[100.0, 200.0], [120.0, 220.0]])
        );
        check_eq!(ex_def["surface"]["floor"], json!(["structure.hub", 3]));

        let syn_def = &doc["entries"]["room.synth"]["def"];
        check_eq!(syn_def["name"], json!("Synthesized Room"));
        check_eq!(syn_def["kind"], json!("circulation"));
        check_eq!(
            syn_def["footprint"]["rect"],
            json!([[40.0, 50.0], [65.0, 80.0]])
        );
        check_eq!(syn_def["surface"]["floor"], json!(["structure.hub", 2]));

        // Pin exact cross-language fixture consumed by Unity NUnit (`TacSimEditModeTests.cs`)
        let Some(projected_str) = project_pack_value_for_client(doc) else {
            return crate::test_checks::failed("projection failed");
        };
        let projected_val: Value = serde_json::from_str(&projected_str)?;
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../unity/Assets/_TacSim/Scripts/Tests/Fixtures~/reconciled_neighborhood.json",
        );
        if std::env::var_os("TACSIM_BLESS").is_some() {
            let mut text = serde_json::to_string_pretty(&projected_val)?;
            text.push('\n');
            std::fs::write(&fixture_path, text)?;
        }
        let disk_val: Value = serde_json::from_str(&std::fs::read_to_string(&fixture_path)?)?;
        check_eq!(disk_val, projected_val);
        Ok(())
    }

    #[test]
    fn high2_world_content_fingerprint_tracks_roomgeo_changes_and_respects_disabled_store()
    -> TestResult {
        use std::hash::Hasher as _;

        use mush_atlas::{
            Access, AtlasState, Footprint, Point2, RecordKey, Rect2, RoomGeo, RoomKind, SurfaceRef,
        };
        use mush_core::CanonicalId;

        let fp_of = |w: &World, atlas: &AtlasState| {
            let mut h1 = std::collections::hash_map::DefaultHasher::new();
            let mut h2: u64 = 0xcbf2_9ce4_8422_2325;
            hash_atlas_state(&mut h1, &mut h2, w, atlas);
            WorldContentFingerprint(h1.finish(), h2)
        };

        let mut tw = mush_testkit::TestWorld::new();
        let room = tw
            .world_mut()
            .create(mush_core::ObjKind::Room, "Bay Alpha", ObjId(1), None)?;
        let pack_id = CanonicalId::new("neighborhood.fp_test")?;
        let nb_key = RecordKey::new(pack_id.clone(), CanonicalId::new("neighborhood.main")?);
        let st_key = RecordKey::new(pack_id.clone(), CanonicalId::new("structure.hub")?);

        let mut atlas = AtlasState::default();
        atlas.records.roomgeo.insert(
            room,
            RoomGeo {
                room,
                key: RecordKey::new(pack_id.clone(), CanonicalId::new("room.bay")?),
                neighborhood: nb_key,
                site: None,
                surface: SurfaceRef::Floor {
                    structure: st_key.clone(),
                    index: 1,
                },
                footprint: Footprint::Rect(Rect2(Point2(0.0, 0.0), Point2(10.0, 10.0))),
                kind: RoomKind::Indoor,
                access: Access::Public,
                revision: 1,
            },
        );

        let fp_initial = fp_of(&tw.engine().world, &atlas);
        check_eq!(fp_initial, fp_of(&tw.engine().world, &atlas));

        // Mutating roomgeo footprint must change the non-allocating fingerprint.
        if let Some(geo) = atlas.records.roomgeo.get_mut(&room) {
            geo.footprint = Footprint::Rect(Rect2(Point2(0.0, 0.0), Point2(25.0, 10.0)));
        }
        let fp_after_footprint = fp_of(&tw.engine().world, &atlas);
        check!(fp_after_footprint != fp_initial);

        // Mutating roomgeo floor surface must also invalidate the fingerprint.
        if let Some(geo) = atlas.records.roomgeo.get_mut(&room) {
            geo.surface = SurfaceRef::Floor {
                structure: st_key,
                index: 2,
            };
        }
        let fp_after_surface = fp_of(&tw.engine().world, &atlas);
        check!(fp_after_surface != fp_after_footprint);

        // Disabled TacsimStore (registered via TacsimRuleset) must short-circuit
        // `tacsim.content` queries with `enabled: false` and stay stable across repeated fingerprints.
        let disabled_cfg = crate::config::TacsimConfig {
            enabled: false,
            ..Default::default()
        };
        let tw_disabled = mush_testkit::TestWorld::with_modules(vec![Box::new(
            crate::TacsimRuleset::with_config(disabled_cfg),
        )]);
        let fp_disabled = world_content_fingerprint(&tw_disabled.engine().world, ObjId(1));
        check_eq!(
            fp_disabled,
            world_content_fingerprint(&tw_disabled.engine().world, ObjId(1))
        );
        let q = ClientContentQuery::new();
        let Ok(OutputEvent::Panel { data, .. }) = q.query(
            &tw_disabled.engine().world,
            ObjId(1),
            &json!({"op": "manifest"}),
        ) else {
            return crate::test_checks::failed("expected panel response");
        };
        check_eq!(data["enabled"], json!(false));
        check_eq!(data["packs"], json!([]));

        let Ok(OutputEvent::Panel {
            data: pack_data, ..
        }) = q.query(
            &tw_disabled.engine().world,
            ObjId(1),
            &json!({"op": "pack", "id": "dray"}),
        )
        else {
            return crate::test_checks::failed("expected pack panel response");
        };
        check_eq!(pack_data["enabled"], json!(false));
        check_eq!(pack_data["found"], json!(false));
        Ok(())
    }
}
