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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};

use mush_core::module::{ModError, Registrar};
use mush_core::{Flags, HookReadContext, ObjId, ObjKind, QueryHandler, World};
use mush_economy::{Amount, EconomicClock, EconomicDeadline, EconomyStore, InstrumentId};
use mush_market::{ListingId, MARKET_PAGE, MarketFilter, MarketStore, Supply};
use mush_org::PrincipalId;
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::definitions::content::{
    CAST_SOURCES, INDUSTRY_HULLS_JSON, LANE_JSON, MARS_CANYON_ATLAS_JSON,
    PROVING_GROUND_ATLAS_JSON, SITE_SOURCES, STATION_SOURCES, WIDE_SITE_SOURCES,
    WINTER_VALLEY_ATLAS_JSON, fielded_sources,
};
use crate::definitions::goods::GOODS_JSON;
use crate::economy::CREDIT;
use crate::port::board::money;
use crate::port::coop::{EnrolmentStatus, name_of, port_name};
use crate::port::credit::own_account;
use crate::port::market::{PortVenueGate, port_of_venue};
use crate::port::ship::{
    Berth, berth_gallery_json, berth_of, hull_of_room, hull_stowage_json, hull_view,
    resolve_actor_hull, simulator_lobby_json,
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
            | "space_mount"
            | "space_fit"
            | "recipe"
    )
}

fn entry_is_staff_or_hidden(kind: &str, def: Option<&Map<String, Value>>) -> bool {
    // Default-deny: drop secrets, lore, quests, hooks, deposits, facilities, crew_offers,
    // passengers, relay_networks, and any non-allowlisted entry kind.
    if !is_allowed_client_entry_kind(kind) {
        return true;
    }
    let Some(def) = def else {
        return false;
    };
    if kind == "room" && def.get("access").and_then(Value::as_str) == Some("restricted") {
        return true;
    }
    if def.get("reveal").and_then(Value::as_str) == Some("staff") {
        return true;
    }
    if def.get("lock").and_then(Value::as_str) == Some("staff") {
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

fn restricted_room_keys_in_doc(doc: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(entries) = doc.get("entries").and_then(Value::as_object) else {
        return out;
    };
    for (key, entry) in entries {
        if entry.get("kind").and_then(Value::as_str) == Some("room")
            && entry
                .get("def")
                .and_then(Value::as_object)
                .and_then(|d| d.get("access"))
                .and_then(Value::as_str)
                == Some("restricted")
        {
            out.insert(key.clone());
        }
    }
    out
}

fn ref_targets_restricted(val: &Value, restricted: &BTreeSet<String>) -> bool {
    if let Some(s) = val.as_str() {
        return restricted.contains(s);
    }
    if let Some(obj) = val.as_object()
        && let Some(r) = obj.get("room").and_then(Value::as_str)
    {
        return restricted.contains(r);
    }
    false
}

fn entry_refs_restricted_room(def: &Map<String, Value>, restricted: &BTreeSet<String>) -> bool {
    if restricted.is_empty() {
        return false;
    }
    ["room", "home", "from", "to"].iter().any(|field| {
        def.get(*field)
            .is_some_and(|v| ref_targets_restricted(v, restricted))
    })
}

fn sanitize_entry_in_place(kind: &str, def: &mut Map<String, Value>) {
    match kind {
        "region" => {
            // Strip authoring prose/style `theme` guidance; retain planetary/surface chart geometry.
            const ALLOWED_REGION_KEYS: &[&str] = &[
                "name",
                "kind",
                "body",
                "extent",
                "datum_m",
                "background",
                "ground_resolution_cm",
            ];
            def.retain(|k, _| ALLOWED_REGION_KEYS.contains(&k.as_str()));
        }
        "space_mount" => {
            const ALLOWED_SPACE_MOUNT_KEYS: &[&str] = &["label", "family"];
            def.retain(|k, _| ALLOWED_SPACE_MOUNT_KEYS.contains(&k.as_str()));
        }
        "space_fit" => {
            const ALLOWED_SPACE_FIT_KEYS: &[&str] =
                &["label", "unit", "pixel_model", "decoys", "rcs", "mounts"];
            def.retain(|k, _| ALLOWED_SPACE_FIT_KEYS.contains(&k.as_str()));
        }
        "recipe" => {
            const ALLOWED_RECIPE_KEYS: &[&str] = &[
                "label",
                "inputs",
                "outputs",
                "output_lots",
                "requires",
                "work_units",
            ];
            def.retain(|k, _| ALLOWED_RECIPE_KEYS.contains(&k.as_str()));
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
                        cobj.retain(|k, _| {
                            matches!(
                                k.as_str(),
                                "id" | "label" | "zone" | "anchor_body" | "rocks"
                            )
                        });
                        if let Some(rocks) = cobj.get_mut("rocks").and_then(Value::as_array_mut) {
                            for rock in rocks {
                                if let Some(robj) = rock.as_object_mut() {
                                    // Preserve real `ceres_phobos.json` public chart fields while
                                    // stripping hidden reserve tonnage (`gross_t`) (CS-H2).
                                    robj.retain(|k, _| {
                                        matches!(
                                            k.as_str(),
                                            "letter"
                                                | "alias"
                                                | "spectral"
                                                | "pos_m"
                                                | "vel_mmps"
                                                | "tenure"
                                                | "holder_org"
                                        )
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

fn project_pack_value_for_client_with_restricted(
    mut doc: Value,
    extra_restricted: &BTreeSet<String>,
) -> Option<String> {
    let mut restricted_rooms = restricted_room_keys_in_doc(&doc);
    restricted_rooms.extend(extra_restricted.iter().cloned());

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
                let hidden = entry_is_staff_or_hidden(kind, def)
                    || def.is_some_and(|d| entry_refs_restricted_room(d, &restricted_rooms));
                hidden.then_some(key.clone())
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

fn project_pack_value_for_client(doc: Value) -> Option<String> {
    project_pack_value_for_client_with_restricted(doc, &BTreeSet::new())
}

/// Project an authoring JSON pack down to player-safe client geometry and public
/// metadata, stripping `"brief"`, `"digest"`, `"secret"`/`"lore"`/`"quest"`
/// entries, `"reveal": "staff"`, `"lock": "staff"`, `"visibility": "hidden"` exits,
/// restricted rooms and any items/exits/NPCs referencing them, guarded NPC `def.lore`,
/// and private NPC prompt fields.
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
    restricted_by_nb: BTreeMap<String, BTreeSet<String>>,
}

impl PackCollector {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            pack_rows: Vec::new(),
            by_id: BTreeMap::new(),
            restricted_by_nb: BTreeMap::new(),
        }
    }

    fn record_restricted_rooms(&mut self, nb_id: &str, doc: &Value) {
        let rooms = restricted_room_keys_in_doc(doc);
        if !rooms.is_empty() {
            self.restricted_by_nb
                .entry(nb_id.to_owned())
                .or_default()
                .extend(rooms);
        }
    }

    fn restricted_rooms_for(&self, nb_id: &str) -> BTreeSet<String> {
        self.restricted_by_nb
            .get(nb_id)
            .cloned()
            .unwrap_or_default()
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
        self.push_projected_value_with_restricted(id, category, doc, aliases, &BTreeSet::new());
    }

    fn push_projected_value_with_restricted(
        &mut self,
        id: &str,
        category: &'static str,
        doc: Value,
        aliases: &[&str],
        extra_restricted: &BTreeSet<String>,
    ) {
        if self.by_id.contains_key(id) {
            return;
        }
        let Some(projected) = project_pack_value_for_client_with_restricted(doc, extra_restricted)
        else {
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
        self.record_restricted_rooms(&id, &doc);
        let short = id.strip_prefix("neighborhood.").unwrap_or("");
        self.push_projected_value(&id, "neighborhood", doc, &[short]);
    }

    fn push_manifest_region_if_applicable(&mut self, raw_json: &str, is_staff_viewer: bool) {
        let Ok(doc) = serde_json::from_str::<Value>(raw_json) else {
            return;
        };
        let Some(id) = doc
            .get("manifest")
            .and_then(Value::as_object)
            .filter(|m| m.get("kind").and_then(Value::as_str) == Some("region"))
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return;
        };
        if !is_staff_viewer && id == "region.limbo" {
            return;
        }
        self.push_projected_value(&id, "region", doc, &[]);
    }

    fn push_fielded_value_if_client_relevant(&mut self, doc: Value) {
        let manifest_id = doc
            .get("manifest")
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let (units, weapons, arenas, has_space_mount, has_space_fit) = {
            let Some(entries) = doc.get("entries").and_then(Value::as_object) else {
                return;
            };
            let mut u = Vec::new();
            let mut wp = Vec::new();
            let mut ar = Vec::new();
            let mut sm = false;
            let mut sf = false;
            for (entry_id, entry) in entries {
                match entry.get("kind").and_then(Value::as_str) {
                    Some("unit") => u.push(entry_id.clone()),
                    Some("weapon") => wp.push(entry_id.clone()),
                    Some("arena") => ar.push(entry_id.clone()),
                    Some("space_mount") => sm = true,
                    Some("space_fit") => sf = true,
                    _ => {}
                }
            }
            (u, wp, ar, sm, sf)
        };
        if let Some(first_unit) = units.first() {
            let slug = first_unit
                .strip_prefix("tacsim.unit.")
                .unwrap_or(first_unit)
                .to_owned();
            let mut aliases: Vec<&str> = units.iter().map(String::as_str).collect();
            if !manifest_id.is_empty() {
                aliases.push(&manifest_id);
            }
            self.push_projected_value(&slug, "unit", doc, &aliases);
        } else if let Some(first_weapon) = weapons.first() {
            let slug = first_weapon
                .strip_prefix("tacsim.weapon.")
                .unwrap_or(first_weapon)
                .to_owned();
            let mut aliases: Vec<&str> = weapons.iter().map(String::as_str).collect();
            if !manifest_id.is_empty() {
                aliases.push(&manifest_id);
            }
            self.push_projected_value(&slug, "weapon", doc, &aliases);
        } else if let Some(first_arena) = arenas.first() {
            let slug = first_arena
                .strip_prefix("tacsim.arena.")
                .unwrap_or(first_arena);
            let client_id = format!("arena.{slug}");
            let mut aliases: Vec<&str> = arenas.iter().map(String::as_str).collect();
            if !manifest_id.is_empty() {
                aliases.push(&manifest_id);
            }
            self.push_projected_value(&client_id, "arena", doc, &aliases);
        } else if has_space_mount {
            self.push_projected_value(
                "space_mounts",
                "space",
                doc,
                &[manifest_id.as_str(), "mounts"],
            );
        } else if has_space_fit {
            self.push_projected_value("space_fits", "space", doc, &[manifest_id.as_str(), "fits"]);
        }
    }

    fn push_fielded_pack_if_client_relevant(&mut self, raw_json: &str) {
        let Ok(doc) = serde_json::from_str::<Value>(raw_json) else {
            return;
        };
        self.push_fielded_value_if_client_relevant(doc);
    }
}

fn exit_spec_from_obj(obj: &mush_core::Obj) -> String {
    if obj.aliases.is_empty() {
        obj.name.clone()
    } else {
        let mut spec = obj.name.clone();
        for alias in &obj.aliases {
            spec.push(';');
            spec.push_str(alias.as_str());
        }
        spec
    }
}

fn exit_lock_label(obj: &mush_core::Obj) -> Option<String> {
    let expr = obj.locks.get(&mush_core::lock::LockType::Basic)?;
    let rendered = expr.to_string();
    let upper = rendered.to_ascii_uppercase();
    if upper.contains("FLAG^WIZARD") || upper.contains("FLAG^ROYALTY") {
        Some("staff".to_owned())
    } else {
        Some(rendered)
    }
}

/// Reconcile a serialized neighborhood document with the live world's applied
/// [`mush_atlas::Records`] (`RoomGeo` footprints/floors/names, `PortalRec`
/// endpoints/commands/geometry, `NeighborhoodRec`, and `StructureRec` floors),
/// so worlds running an older applied revision, off-shelf record recovery, or
/// live geometry edits receive exact matching room bounds, room sets, and
/// walkable public portals.
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
    let live_portals: BTreeMap<String, &mush_atlas::PortalRec> = records
        .portals
        .iter()
        .filter(|(k, _)| &k.pack == pack_id)
        .map(|(k, p)| (k.entry.as_str().to_owned(), p))
        .collect();
    let has_live_neighborhood = records.neighborhoods.keys().any(|k| &k.pack == pack_id);
    if live_rooms.is_empty() && !has_live_neighborhood && live_portals.is_empty() {
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

    // When live portal records exist for this pack, prune unapplied shelf exits so
    // the reconstructed exit graph matches the applied revision.
    if !live_portals.is_empty() {
        let unapplied_exits: Vec<String> = entries
            .iter()
            .filter_map(|(k, v)| {
                let is_exit = v.get("kind").and_then(Value::as_str) == Some("exit");
                (is_exit && !live_portals.contains_key(k)).then_some(k.clone())
            })
            .collect();
        for k in unapplied_exits {
            entries.remove(&k);
        }
    }

    // Overlay live RoomGeo footprint (`def.footprint.rect`), surface (`def.surface.floor`),
    // room kind, access, and live room object name using the exact `mush-atlas` pack wire schema
    // consumed by Unity's `TacSimContentLoader.ParseNeighborhoodJson`.
    let mut room_entry_by_obj: BTreeMap<ObjId, &str> = BTreeMap::new();
    for (entry_key, geo) in &live_rooms {
        room_entry_by_obj.insert(geo.room, entry_key.as_str());
        let row = entries
            .entry(entry_key.clone())
            .or_insert_with(|| json!({ "kind": "room", "def": {} }));
        if let Some(def) = row.get_mut("def").and_then(Value::as_object_mut) {
            if let Some(obj) = w.get(geo.room) {
                def.insert("name".to_owned(), Value::String(obj.name.to_string()));
                if obj.markers.contains("port.dock") {
                    let markers = def
                        .entry("markers".to_owned())
                        .or_insert_with(|| json!([]))
                        .as_array_mut();
                    if let Some(arr) = markers
                        && !arr.iter().any(|m| m.as_str() == Some("port.dock"))
                    {
                        arr.push(json!("port.dock"));
                    }
                }
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

    // Reconstruct live PortalRec endpoints, exit commands/aliases, locks, and portal geometry
    // from the applied revision (critical when recovering an off-shelf neighborhood from
    // `atlas.records`, and to keep shelf-present packs aligned with live edits).
    let resolve_endpoint = |obj_id: ObjId| -> Option<(String, bool)> {
        if obj_id == mush_core::LIMBO {
            return Some(("limbo".to_owned(), false));
        }
        if let Some(entry_key) = room_entry_by_obj.get(&obj_id) {
            return Some(((*entry_key).to_owned(), false));
        }
        let geo = records.roomgeo.get(&obj_id)?;
        let cross_restricted = geo.access != mush_atlas::Access::Public;
        Some((
            format!("{}.{}", geo.key.pack, geo.key.entry),
            cross_restricted,
        ))
    };

    for (entry_key, portal) in &live_portals {
        let (
            mush_atlas::record::Endpoint::Room(from_obj),
            mush_atlas::record::Endpoint::Room(to_obj),
        ) = (portal.from, portal.to)
        else {
            entries.remove(entry_key);
            continue;
        };
        let (Some((mut from_key, from_cross_restricted)), Some((mut to_key, to_cross_restricted))) =
            (resolve_endpoint(from_obj), resolve_endpoint(to_obj))
        else {
            entries.remove(entry_key);
            continue;
        };

        let mut forward_obj: Option<&mush_core::Obj> = None;
        let mut back_obj: Option<&mush_core::Obj> = None;
        for &exit_id in &portal.exits {
            let Some(obj) = w.get(exit_id) else {
                continue;
            };
            if obj.kind != ObjKind::Exit || obj.flags.contains(Flags::GOING) {
                continue;
            }
            if obj.location == Some(from_obj) && obj.destination == Some(to_obj) {
                forward_obj = Some(obj);
            } else if obj.location == Some(to_obj) && obj.destination == Some(from_obj) {
                back_obj = Some(obj);
            } else if forward_obj.is_none() {
                forward_obj = Some(obj);
            } else if back_obj.is_none() {
                back_obj = Some(obj);
            }
        }

        let mut names = Vec::with_capacity(2);
        let mut dz_m = portal.dz_m;
        match (forward_obj, back_obj) {
            (Some(fwd), Some(bck)) => {
                names.push(Value::String(exit_spec_from_obj(fwd)));
                names.push(Value::String(exit_spec_from_obj(bck)));
            }
            (Some(fwd), None) => {
                names.push(Value::String(exit_spec_from_obj(fwd)));
            }
            (None, Some(bck)) => {
                std::mem::swap(&mut from_key, &mut to_key);
                dz_m = -dz_m;
                names.push(Value::String(exit_spec_from_obj(bck)));
            }
            (None, None) => {}
        }
        let existing_has_names = entries
            .get(entry_key)
            .and_then(|r| r.get("def"))
            .and_then(|d| d.get("names"))
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        if names.is_empty() && !existing_has_names {
            entries.remove(entry_key);
            continue;
        }

        let is_hidden = portal.visibility == mush_atlas::record::Visibility::Hidden
            || from_cross_restricted
            || to_cross_restricted
            || forward_obj.is_some_and(|o| o.flags.contains(Flags::DARK))
            || back_obj.is_some_and(|o| o.flags.contains(Flags::DARK));
        let lock_label = forward_obj
            .and_then(exit_lock_label)
            .or_else(|| back_obj.and_then(exit_lock_label));

        let mut portal_obj = Map::new();
        portal_obj.insert("mode".to_owned(), json!(portal.mode.label()));
        portal_obj.insert(
            "visibility".to_owned(),
            json!(if is_hidden { "hidden" } else { "public" }),
        );
        if dz_m != 0.0 {
            portal_obj.insert("dz_m".to_owned(), json!(dz_m));
        }
        if portal.distance_m > 0 {
            portal_obj.insert("distance_m".to_owned(), json!(portal.distance_m));
        }
        match portal.geometry {
            mush_atlas::record::PortalGeometry::Segment(a, b) => {
                portal_obj.insert("at".to_owned(), json!([[a.0, a.1, a.2], [b.0, b.1, b.2]]));
            }
            mush_atlas::record::PortalGeometry::Point(p) => {
                portal_obj.insert("at".to_owned(), json!([p.0, p.1, p.2]));
            }
        }

        let row = entries
            .entry(entry_key.clone())
            .or_insert_with(|| json!({ "kind": "exit", "def": {} }));
        if let Some(def) = row.get_mut("def").and_then(Value::as_object_mut) {
            def.insert("from".to_owned(), Value::String(from_key));
            def.insert("to".to_owned(), Value::String(to_key));
            if !names.is_empty() {
                def.insert("names".to_owned(), Value::Array(names));
            }
            def.insert("portal".to_owned(), Value::Object(portal_obj));
            if let Some(lock) = lock_label {
                def.insert("lock".to_owned(), Value::String(lock));
            } else if forward_obj.is_some() || back_obj.is_some() {
                def.remove("lock");
            }
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

    // 1. Live world neighborhoods, regions & arena atlases from `AtlasState` (`shelf` + `records`)
    if let Some(atlas) = atlas {
        let mut pending_casts: Vec<(String, String, Value)> = Vec::new();
        for (id, pack) in &atlas.shelf {
            if let Some(content) = content {
                let Some(applied) = content.latches.applied(id) else {
                    continue;
                };
                if applied.is_retired() {
                    continue;
                }
            }
            if pack.manifest().kind() == mush_content::PackKind::Region {
                if let Ok(doc) = serde_json::to_value(pack) {
                    let has_heightfield = doc
                        .get("entries")
                        .and_then(Value::as_object)
                        .is_some_and(|e| {
                            e.values().any(|row| {
                                row.get("kind").and_then(Value::as_str) == Some("heightfield")
                            })
                        });
                    if has_heightfield {
                        let arena_slug = id.as_str().strip_prefix("tacsim.").unwrap_or(id.as_str());
                        let client_id = format!("arena_atlas.{arena_slug}");
                        col.push_projected_value(&client_id, "arena_atlas", doc, &[id.as_str()]);
                    } else if is_staff_viewer || id.as_str() != "region.limbo" {
                        col.push_projected_value(id.as_str(), "region", doc, &[]);
                    }
                }
                continue;
            }
            if pack.manifest().kind() == mush_content::PackKind::Neighborhood {
                if let Ok(mut doc) = serde_json::to_value(pack) {
                    reconcile_neighborhood_with_live_records(w, &atlas.records, id, &mut doc);
                    if !is_staff_viewer && !neighborhood_is_player_accessible(&doc) {
                        continue;
                    }
                    col.record_restricted_rooms(id.as_str(), &doc);
                    let short = id.as_str().strip_prefix("neighborhood.").unwrap_or("");
                    col.push_projected_value(id.as_str(), "neighborhood", doc, &[short]);
                }
            } else if let Some(cast_slug) = id.as_str().strip_prefix("tacsim.cast.")
                && let Ok(doc) = serde_json::to_value(pack)
            {
                pending_casts.push((id.as_str().to_owned(), cast_slug.to_owned(), doc));
            }
        }

        // Synthesize any applied neighborhood present in `atlas.records` whose source
        // pack is no longer on `atlas.shelf`.
        let mut record_packs = BTreeSet::new();
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
            col.record_restricted_rooms(pack_id.as_str(), &doc);
            let short = pack_id.as_str().strip_prefix("neighborhood.").unwrap_or("");
            col.push_projected_value(pack_id.as_str(), "neighborhood", doc, &[short]);
        }

        // Gate `tacsim.cast.*` packs on companion neighborhood accessibility and drop NPCs
        // stationed in companion restricted rooms (CS-H1).
        for (raw_id, cast_slug, doc) in pending_casts {
            let nb_id = format!("neighborhood.{cast_slug}");
            if !is_staff_viewer && !col.by_id.contains_key(&nb_id) {
                continue;
            }
            let restricted = col.restricted_rooms_for(&nb_id);
            let client_id = format!("cast.{cast_slug}");
            col.push_projected_value_with_restricted(
                &client_id,
                "cast",
                doc,
                &[raw_id.as_str()],
                &restricted,
            );
        }
    }

    // 2. Station public cast fallback when cast packs were seeded outside `atlas.shelf`
    for &(pack_id, json_src) in &CAST_SOURCES {
        let Ok(cid) = mush_core::CanonicalId::new(pack_id) else {
            continue;
        };
        if let Some(content) = content
            && content
                .latches
                .applied(&cid)
                .is_some_and(mush_content::Applied::is_retired)
        {
            continue;
        }
        let slug = pack_id.strip_prefix("tacsim.cast.").unwrap_or(pack_id);
        let nb_id = format!("neighborhood.{slug}");
        // Always gate non-staff cast packs on companion neighborhood accessibility (CS-H1).
        if !is_staff_viewer && !col.by_id.contains_key(&nb_id) {
            continue;
        }
        if let Ok(doc) = serde_json::from_str::<Value>(json_src) {
            let restricted = col.restricted_rooms_for(&nb_id);
            let client_id = format!("cast.{slug}");
            col.push_projected_value_with_restricted(
                &client_id,
                "cast",
                doc,
                &[pack_id],
                &restricted,
            );
        }
    }

    // 3. Authoritative `GoodsTable` (standalone typed document outside `TacsimStore::packs()`)
    if let Ok(table) = crate::definitions::goods::goods_table()
        && let Ok(goods_doc) = serde_json::to_value(table)
    {
        col.push_projected_value("goods", "market", goods_doc, &["tacsim.goods"]);
    } else {
        col.push_projected("goods", "market", GOODS_JSON, &["tacsim.goods"]);
    }

    // 4. Live `TacsimStore` rules, arena definitions, & campaign-seed packs at world-applied
    //    or published versions (never unapplied staged drafts or withdrawn designs; CS-H1)
    if let Some(store) = store {
        let mut unique_ids = BTreeSet::new();
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
            // Require applied status for CampaignSeed packs (including Arenas) and applied or
            // published status for RulesDefinition packs; never fall back to unapplied
            // `newest_loaded` (CS-H1).
            let active_ver = store.applied_in(w, &id).or_else(|| {
                store
                    .versions_of(&id)
                    .filter(|&v| {
                        store.published_digest(&id, v).is_some()
                            && store.pack(&id, v).is_some_and(|lp| {
                                lp.pack.manifest().kind() == mush_content::PackKind::RulesDefinition
                            })
                    })
                    .max()
            });
            let Some(ver) = active_ver else {
                continue;
            };
            let Some(loaded) = store.pack(&id, ver) else {
                continue;
            };
            let Ok(mut doc) = loaded.pack.canonical_value() else {
                continue;
            };
            // Drop any withdrawn unit designs (`store.design_withdrawn`) from the projected pack.
            if let Some(entries) = doc.get_mut("entries").and_then(Value::as_object_mut) {
                let withdrawn: Vec<String> = entries
                    .iter()
                    .filter_map(|(entry_id, row)| {
                        let is_unit = row.get("kind").and_then(Value::as_str) == Some("unit");
                        (is_unit && store.design_withdrawn(entry_id)).then_some(entry_id.clone())
                    })
                    .collect();
                for key in withdrawn {
                    entries.remove(&key);
                }
            }
            let has_fielded_def = doc
                .get("entries")
                .and_then(Value::as_object)
                .is_some_and(|e| {
                    e.values().any(|row| {
                        matches!(
                            row.get("kind").and_then(Value::as_str),
                            Some("unit" | "weapon" | "arena" | "space_mount" | "space_fit")
                        )
                    })
                });
            if has_fielded_def {
                col.push_fielded_value_if_client_relevant(doc);
            } else if id.as_str() == "tacsim.industry.hulls" {
                col.push_projected_value(
                    "hulls",
                    "industry",
                    doc,
                    &["tacsim.industry.hulls", "industry.hulls", "hulls-v2"],
                );
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

        // Fill in stock `Catalog` unit/weapon/arena/space definitions (`fielded_sources` + conduct
        // carrier) that were compiled into `store.definitions()` rather than repeated in
        // `TacsimConfig.packs`, skipping any pack overridden above or retired in `ContentState`.
        for raw_src in fielded_sources()
            .into_iter()
            .map(|s| s.bytes)
            .chain(std::iter::once(CONDUCT_CARRIER_JSON.to_owned()))
        {
            let Ok(mut doc) = serde_json::from_str::<Value>(&raw_src) else {
                continue;
            };
            let manifest_id = doc
                .get("manifest")
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if !manifest_id.is_empty() {
                if col.by_id.contains_key(&manifest_id) {
                    continue;
                }
                if let Ok(cid) = mush_core::CanonicalId::new(&manifest_id)
                    && content.is_some_and(|c| {
                        c.latches
                            .applied(&cid)
                            .is_some_and(mush_content::Applied::is_retired)
                    })
                {
                    continue;
                }
            }
            if let Some(entries) = doc.get_mut("entries").and_then(Value::as_object_mut) {
                let withdrawn: Vec<String> = entries
                    .iter()
                    .filter_map(|(entry_id, row)| {
                        let is_unit = row.get("kind").and_then(Value::as_str) == Some("unit");
                        (is_unit && store.design_withdrawn(entry_id)).then_some(entry_id.clone())
                    })
                    .collect();
                for key in withdrawn {
                    entries.remove(&key);
                }
            }
            col.push_fielded_value_if_client_relevant(doc);
        }

        // Yard hull recipes (`industry/hulls-v2.json`), if not overridden above or retired.
        let hulls_retired = mush_core::CanonicalId::new("tacsim.industry.hulls")
            .ok()
            .is_some_and(|cid| {
                content.is_some_and(|c| {
                    c.latches
                        .applied(&cid)
                        .is_some_and(mush_content::Applied::is_retired)
                })
            });
        if !hulls_retired && !col.by_id.contains_key("hulls") {
            col.push_projected(
                "hulls",
                "industry",
                INDUSTRY_HULLS_JSON,
                &["tacsim.industry.hulls", "industry.hulls", "hulls-v2"],
            );
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

        // 2. All station, surface site, wide site, and relay neighborhoods & regions (`tacsim`)
        for &(_id, json_src) in STATION_SOURCES
            .iter()
            .chain(SITE_SOURCES.iter())
            .chain(WIDE_SITE_SOURCES.iter())
        {
            col.push_manifest_neighborhood_if_applicable(json_src);
            col.push_manifest_region_if_applicable(json_src, false);
        }
        for json_src in crate::relay_content::GEOGRAPHY {
            col.push_manifest_neighborhood_if_applicable(json_src);
            col.push_manifest_region_if_applicable(json_src, false);
        }

        // 3. Station public cast metadata (`cast.<slug>`, with lore, prompt internals,
        //    and restricted-room NPCs stripped)
        for &(pack_id, json_src) in &CAST_SOURCES {
            let slug = pack_id.strip_prefix("tacsim.cast.").unwrap_or(pack_id);
            let nb_id = format!("neighborhood.{slug}");
            let restricted = col.restricted_rooms_for(&nb_id);
            let client_id = format!("cast.{slug}");
            if let Ok(doc) = serde_json::from_str::<Value>(json_src) {
                col.push_projected_value_with_restricted(
                    &client_id,
                    "cast",
                    doc,
                    &[pack_id],
                    &restricted,
                );
            }
        }

        // 4. Economy goods table, yard hull recipes, & trade corridor lane
        col.push_projected("goods", "market", GOODS_JSON, &["tacsim.goods"]);
        col.push_projected(
            "hulls",
            "industry",
            INDUSTRY_HULLS_JSON,
            &["tacsim.industry.hulls", "industry.hulls", "hulls-v2"],
        );
        col.push_projected(
            "ceres_phobos",
            "lane",
            LANE_JSON,
            &["tacsim.lane.ceres_phobos"],
        );

        // 5. All fielded units, weapons, space mounts/fits, and arena definitions
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

        col.finish()
    })
}

fn hash_footprint(h: &mut Sha256, fp: &mush_atlas::Footprint) {
    match fp {
        mush_atlas::Footprint::Rect(mush_atlas::Rect2(
            mush_atlas::Point2(x0, y0),
            mush_atlas::Point2(x1, y1),
        )) => {
            h.update([1u8]);
            h.update(x0.to_bits().to_le_bytes());
            h.update(y0.to_bits().to_le_bytes());
            h.update(x1.to_bits().to_le_bytes());
            h.update(y1.to_bits().to_le_bytes());
        }
        mush_atlas::Footprint::Polygon(mush_atlas::Polygon2(pts)) => {
            h.update([2u8]);
            h.update((pts.len() as u64).to_le_bytes());
            for mush_atlas::Point2(px, py) in pts {
                h.update(px.to_bits().to_le_bytes());
                h.update(py.to_bits().to_le_bytes());
            }
        }
        mush_atlas::Footprint::Anchor { point, radius_m } => {
            h.update([3u8]);
            h.update(point.0.to_bits().to_le_bytes());
            h.update(point.1.to_bits().to_le_bytes());
            h.update(radius_m.to_bits().to_le_bytes());
        }
    }
}

fn hash_atlas_records(h: &mut Sha256, w: &World, records: &mush_atlas::Records) {
    h.update(b"|geo:");
    for (room_id, geo) in &records.roomgeo {
        h.update(room_id.0.to_le_bytes());
        h.update(geo.revision.to_le_bytes());
        h.update(geo.key.pack.as_str().as_bytes());
        h.update(b":");
        h.update(geo.key.entry.as_str().as_bytes());
        h.update([match geo.kind {
            mush_atlas::RoomKind::Outdoor => 0,
            mush_atlas::RoomKind::Indoor => 1,
            mush_atlas::RoomKind::Circulation => 2,
        }]);
        h.update([match geo.access {
            mush_atlas::Access::Public => 0,
            mush_atlas::Access::Restricted => 1,
        }]);
        match &geo.surface {
            mush_atlas::SurfaceRef::Ground => h.update([0u8]),
            mush_atlas::SurfaceRef::Floor {
                structure, index, ..
            } => {
                h.update([1u8]);
                h.update(structure.pack.as_str().as_bytes());
                h.update(b":");
                h.update(structure.entry.as_str().as_bytes());
                h.update(index.to_le_bytes());
            }
        }
        if let Some(obj) = w.get(*room_id) {
            h.update(obj.name.as_bytes());
            h.update([u8::from(obj.markers.contains("port.dock"))]);
        }
        hash_footprint(h, &geo.footprint);
    }
    h.update(b"|nb:");
    for (nkey, nb) in &records.neighborhoods {
        h.update(nkey.pack.as_str().as_bytes());
        h.update(b":");
        h.update(nkey.entry.as_str().as_bytes());
        h.update(nb.revision.to_le_bytes());
        h.update(nb.name.as_bytes());
        h.update(nb.region.pack.as_str().as_bytes());
        h.update(nb.setting.as_bytes());
    }
    h.update(b"|st:");
    for (skey, st) in &records.structures {
        h.update(skey.pack.as_str().as_bytes());
        h.update(b":");
        h.update(skey.entry.as_str().as_bytes());
        h.update(st.revision.to_le_bytes());
        h.update((st.footprint.0.len() as u64).to_le_bytes());
        for mush_atlas::Point2(px, py) in &st.footprint.0 {
            h.update(px.to_bits().to_le_bytes());
            h.update(py.to_bits().to_le_bytes());
        }
        h.update((st.floors.len() as u64).to_le_bytes());
        for fl in &st.floors {
            h.update(fl.index.to_le_bytes());
            h.update(fl.z_offset_m.to_bits().to_le_bytes());
            h.update(fl.height_m.to_bits().to_le_bytes());
            if let Some(name) = &fl.name {
                h.update(name.as_bytes());
            }
        }
    }
    h.update(b"|pt:");
    for (pkey, portal) in &records.portals {
        h.update(pkey.pack.as_str().as_bytes());
        h.update(b":");
        h.update(pkey.entry.as_str().as_bytes());
        h.update(portal.revision.to_le_bytes());
        for ep in [portal.from, portal.to] {
            match ep {
                mush_atlas::record::Endpoint::Room(id) => {
                    h.update([1u8]);
                    h.update(id.0.to_le_bytes());
                }
                mush_atlas::record::Endpoint::Outside(p) => {
                    h.update([2u8]);
                    h.update(p.0.to_bits().to_le_bytes());
                    h.update(p.1.to_bits().to_le_bytes());
                    h.update(p.2.to_bits().to_le_bytes());
                }
            }
        }
        h.update(portal.mode.label().as_bytes());
        h.update([u8::from(
            portal.visibility == mush_atlas::record::Visibility::Hidden,
        )]);
        h.update(portal.distance_m.to_le_bytes());
        h.update(portal.dz_m.to_bits().to_le_bytes());
        match portal.geometry {
            mush_atlas::record::PortalGeometry::Segment(a, b) => {
                h.update([1u8]);
                for v in [a.0, a.1, a.2, b.0, b.1, b.2] {
                    h.update(v.to_bits().to_le_bytes());
                }
            }
            mush_atlas::record::PortalGeometry::Point(p) => {
                h.update([2u8]);
                for v in [p.0, p.1, p.2] {
                    h.update(v.to_bits().to_le_bytes());
                }
            }
        }
        h.update((portal.exits.len() as u64).to_le_bytes());
        for &exit_id in &portal.exits {
            h.update(exit_id.0.to_le_bytes());
            if let Some(obj) = w.get(exit_id) {
                h.update([
                    1u8,
                    u8::from(obj.flags.contains(Flags::GOING)),
                    u8::from(obj.flags.contains(Flags::DARK)),
                ]);
                h.update(obj.name.as_bytes());
                h.update(b";");
                for alias in &obj.aliases {
                    h.update(alias.as_str().as_bytes());
                    h.update(b";");
                }
                if let Some(loc) = obj.location {
                    h.update(loc.0.to_le_bytes());
                }
                if let Some(dest) = obj.destination {
                    h.update(dest.0.to_le_bytes());
                }
                if let Some(lock) = exit_lock_label(obj) {
                    h.update(b"|lock:");
                    h.update(lock.as_bytes());
                }
            } else {
                h.update([0u8]);
            }
        }
    }
}

/// Non-allocating revision/geometry fingerprint for [`ClientContentQuery`] Level-2 cache validation.
///
/// Avoids per-query `serde_json::to_vec(&geo.footprint)` heap serialization while invalidating
/// whenever applied latches, atlas packs/records/portals, room geometry/names, or TacSim store
/// enablement/packs/designs change.
fn world_content_fingerprint(w: &World, actor: ObjId) -> [u8; 32] {
    let mut h = Sha256::new();
    let is_staff_viewer = mush_atlas::is_staff(w, actor);
    h.update([u8::from(is_staff_viewer)]);
    if let Some(content) = w.module_state::<mush_content::ContentState>() {
        h.update(b"|content:");
        for applied in content.latches.all() {
            h.update(applied.pack().as_str().as_bytes());
            h.update(b":");
            h.update(applied.version().to_le_bytes());
            h.update(applied.digest().as_bytes());
            h.update([u8::from(applied.is_retired())]);
        }
    }
    if let Some(atlas) = w.module_state::<mush_atlas::AtlasState>() {
        h.update(b"|shelf:");
        for (id, pack) in &atlas.shelf {
            h.update(id.as_str().as_bytes());
            h.update(pack.manifest().version().to_le_bytes());
            h.update(pack.manifest().digest().as_bytes());
        }
        hash_atlas_records(&mut h, w, &atlas.records);
    }
    if let Some(store) = w.module_state::<TacsimStore>() {
        h.update(b"|tacsim:");
        h.update([u8::from(store.enabled())]);
        for loaded in store.packs() {
            let id = loaded.pack.manifest().id();
            let ver = loaded.pack.manifest().version();
            h.update(id.as_str().as_bytes());
            h.update(ver.to_le_bytes());
            h.update(loaded.pack.manifest().digest().as_bytes());
            let applied = store.applied_in(w, id).unwrap_or(0);
            h.update(applied.to_le_bytes());
            let published = u8::from(store.published_digest(id, ver).is_some());
            h.update([published]);
        }
        h.update(b"|designs:");
        for d in store.designs() {
            h.update(d.id.as_bytes());
            h.update(d.version.to_le_bytes());
            h.update([u8::from(d.withdrawn)]);
        }
    }
    h.finalize().into()
}

/// Maximum serialized byte size of a `tacsim.content` or `port.trade` query parameter payload.
const MAX_QUERY_PARAMS_BYTES: usize = 256;
/// Maximum byte length of a requested pack identifier.
const MAX_PACK_ID_BYTES: usize = 128;
/// Maximum outbound keyframe byte size for `tacsim.content`.
const MAX_CONTENT_KEYFRAME_BYTES: usize = 512 * 1024;
/// Maximum outbound keyframe byte size for `port.trade`.
const MAX_TRADE_KEYFRAME_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContentQueryOp {
    Manifest,
    Pack,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentQueryParams {
    #[serde(default)]
    op: Option<ContentQueryOp>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortTradeParams {
    #[serde(default)]
    after: Option<u64>,
}

fn require_live_player(w: &World, actor: ObjId, err_msg: &str) -> Result<(), RichText> {
    match w.get(actor) {
        Some(obj) if obj.kind == ObjKind::Player && !obj.flags.contains(Flags::GOING) => Ok(()),
        _ => Err(RichText::plain(err_msg)),
    }
}

fn decode_closed_params<T: serde::de::DeserializeOwned>(
    w: &World,
    actor: ObjId,
    params: &Value,
    err_msg: &str,
) -> Result<T, RichText> {
    require_live_player(w, actor, err_msg)?;
    let encoded_len = serde_json::to_vec(params).map_or(usize::MAX, |v| v.len());
    if encoded_len > MAX_QUERY_PARAMS_BYTES {
        return Err(RichText::plain(err_msg));
    }
    let normalized = if params.is_null() {
        json!({})
    } else {
        params.clone()
    };
    serde_json::from_value(normalized).map_err(|_| RichText::plain(err_msg))
}

fn check_keyframe_size(data: &Value, max_bytes: usize, err_msg: &str) -> Result<(), RichText> {
    let bytes = serde_json::to_vec(data).map_or(usize::MAX, |v| v.len());
    if bytes > max_bytes {
        return Err(RichText::plain(err_msg));
    }
    Ok(())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContentCacheRev {
    world_id: u64,
    obj_meta_rev: u64,
    content_rev: u64,
    atlas_rev: u64,
    tacsim_rev: u64,
}

impl ContentCacheRev {
    fn capture(w: &World) -> Self {
        Self {
            world_id: w.instance_id(),
            obj_meta_rev: w.obj_meta_rev(),
            content_rev: w.module_state_rev::<mush_content::ContentState>(),
            atlas_rev: w.module_state_rev::<mush_atlas::AtlasState>(),
            tacsim_rev: w.module_state_rev::<TacsimStore>(),
        }
    }
}

struct CachedContentBundle {
    rev: ContentCacheRev,
    fingerprint: [u8; 32],
    bundle: Arc<ClientContentBundle>,
}

struct ClientContentQuery {
    player_cache: Mutex<Option<CachedContentBundle>>,
    staff_cache: Mutex<Option<CachedContentBundle>>,
    #[cfg(test)]
    fingerprint_scans: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    bundle_builds: std::sync::atomic::AtomicU64,
}

impl ClientContentQuery {
    fn new() -> Self {
        Self {
            player_cache: Mutex::new(None),
            staff_cache: Mutex::new(None),
            #[cfg(test)]
            fingerprint_scans: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            bundle_builds: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn bundle_for(&self, w: &World, actor: ObjId) -> Arc<ClientContentBundle> {
        let is_staff = mush_atlas::is_staff(w, actor);
        let slot = if is_staff {
            &self.staff_cache
        } else {
            &self.player_cache
        };
        let rev = ContentCacheRev::capture(w);
        if let Ok(guard) = slot.lock()
            && let Some(cached) = guard.as_ref()
            && cached.rev == rev
        {
            return Arc::clone(&cached.bundle);
        }
        #[cfg(test)]
        self.fingerprint_scans
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let fp = world_content_fingerprint(w, actor);
        if let Ok(mut guard) = slot.lock()
            && let Some(cached) = guard.as_mut()
            && cached.fingerprint == fp
        {
            cached.rev = rev;
            return Arc::clone(&cached.bundle);
        }
        #[cfg(test)]
        self.bundle_builds
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let built = Arc::new(world_content_bundle(w, actor));
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(CachedContentBundle {
                rev,
                fingerprint: fp,
                bundle: Arc::clone(&built),
            });
        }
        built
    }
}

impl QueryHandler for ClientContentQuery {
    fn query(&self, w: &World, actor: ObjId, params: &Value) -> Result<OutputEvent, RichText> {
        const BAD_QUERY: &str = "Invalid content query.";
        let parsed: ContentQueryParams = decode_closed_params(w, actor, params, BAD_QUERY)?;
        let op = match (parsed.op, parsed.id.as_deref()) {
            (None | Some(ContentQueryOp::Manifest), None) => ContentQueryOp::Manifest,
            (Some(ContentQueryOp::Pack), Some(id)) => {
                let trimmed = id.trim();
                if trimmed.is_empty()
                    || id.len() > MAX_PACK_ID_BYTES
                    || id.chars().any(char::is_control)
                {
                    return Err(RichText::plain(BAD_QUERY));
                }
                ContentQueryOp::Pack
            }
            _ => return Err(RichText::plain(BAD_QUERY)),
        };

        if w.module_state::<TacsimStore>()
            .is_some_and(|store| !store.enabled())
        {
            let data = match op {
                ContentQueryOp::Pack => {
                    let req_id = parsed.id.as_deref().unwrap_or("").trim();
                    json!({
                        "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                        "enabled": false,
                        "op": "pack",
                        "found": false,
                        "id": req_id,
                    })
                }
                ContentQueryOp::Manifest => json!({
                    "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                    "enabled": false,
                    "op": "manifest",
                    "packs": [],
                }),
            };
            return Ok(OutputEvent::Panel {
                feed: SmolStr::new_static(CLIENT_CONTENT_FEED),
                kind: PanelFrameKind::Keyframe,
                data,
            });
        }

        let bundle = self.bundle_for(w, actor);
        let data = match op {
            ContentQueryOp::Pack => {
                let req_id = parsed.id.as_deref().unwrap_or("").trim();
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
            }
            ContentQueryOp::Manifest => json!({
                "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                "op": "manifest",
                "packs": bundle.manifest_packs_value.clone(),
            }),
        };
        check_keyframe_size(
            &data,
            MAX_CONTENT_KEYFRAME_BYTES,
            "Content payload too large.",
        )?;
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

fn deadline_lapsed(w: &World, deadline: EconomicDeadline) -> bool {
    match deadline.clock {
        EconomicClock::RealUtc => w.now() >= deadline.at,
        EconomicClock::WorldIc => w.world_ic_now().at >= deadline.at,
        EconomicClock::Manual => false,
        EconomicClock::Cadence(id) => w
            .module_state::<EconomyStore>()
            .and_then(|e| e.snapshot().cadence(id))
            .is_some_and(|c| i64::try_from(c.position).map_or(true, |pos| pos >= deadline.at)),
    }
}

fn build_open_quotes_json(w: &World, market_store: &MarketStore, actor: ObjId) -> Vec<Value> {
    let buyer = PrincipalId::Object(actor);
    let mut out = Vec::new();
    for q in market_store.quotes_of_buyer(&buyer) {
        if q.contract.is_some() || q.settled.is_some() || deadline_lapsed(w, q.expires) {
            continue;
        }
        let Some(l) = market_store.listing(q.listing) else {
            continue;
        };
        if deadline_lapsed(w, l.expires)
            || l.revision != q.observed.listing
            || l.definition_version != q.observed.definition_version
            || l.quantity < q.quantity
            || mush_market::asset_state(w, l) != q.observed.state
        {
            continue;
        }
        if let (Some(src_id), Some(obs_rev)) = (&q.source, q.observed.source)
            && !market_store
                .source(*src_id)
                .is_some_and(|s| s.revision == obs_rev)
        {
            continue;
        }
        let summary = mush_market::summarize_listing(w, l);
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
            "price_text": format_price(w, summary.instrument.as_ref(), q.price),
            "total_text": format_price(w, summary.instrument.as_ref(), Amount::from_minor(total_minor)),
            "expires_at": q.expires.at,
        }));
    }
    out
}

fn build_port_trade_frame(
    ctx: Option<&HookReadContext<'_>>,
    w: &World,
    actor: ObjId,
    after: Option<u64>,
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

    let after_cursor = after.map(ListingId);

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
                let supply_kind = if l.object.is_some() {
                    "asset"
                } else {
                    match &l.source {
                        Supply::Stock { .. } => "stock",
                        Supply::Lot { .. } => "lot",
                        Supply::Wanted { .. } => "wanted",
                        Supply::Market { .. } => "market",
                        Supply::Procurement { .. } => "procurement",
                    }
                };
                let is_exact_lot = l.source.exact_lot().is_some();
                let is_private = l.private_to.is_some();
                let grade = l.source.lot().map(|(_, _, g)| g);
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
                    "supply_kind": supply_kind,
                    "is_exact_lot": is_exact_lot,
                    "is_private": is_private,
                    "grade": grade,
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

    let actor_room = w.get(actor).and_then(|o| o.location);
    let ship_json = resolve_actor_hull(w, actor, actor_room, venue.as_ref()).and_then(|resolved| {
        let view = hull_view(w, resolved.hull)?;
        let (docked, port_entry) = match berth_of(w, resolved.hull) {
            Berth::Docked { map, .. } => (true, Some(map.entry.to_string())),
            Berth::UnderWay | Berth::Elsewhere => (false, None),
        };
        let (sales, buyout_minor) = match resolved.charter_contract {
            Some(contract) => store
                .enrolment_for_lease(contract.0)
                .filter(|e| e.status == EnrolmentStatus::Active)
                .map_or((0_u32, 0_i64), |e| (e.sales, e.buyout_minor)),
            None => (0_u32, 0_i64),
        };
        let stowage = ctx.and_then(|c| hull_stowage_json(c, w, store, &view));
        Some(json!({
            "hull_id": resolved.hull.to_string(),
            "hull_name": name_of(w, resolved.hull),
            "mark": view.row.mark,
            "unit": view.unit.to_string(),
            "class": view.class_word(),
            "docked": docked,
            "port": port_entry,
            "standing": resolved.standing_word,
            "sales": sales,
            "buyout_minor": buyout_minor,
            "buyout_text": money(w, Amount::from_minor(buyout_minor)),
            "stowage": stowage,
        }))
    });

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
        const BAD_QUERY: &str = "Invalid port trade query.";
        let parsed: PortTradeParams = decode_closed_params(w, actor, params, BAD_QUERY)?;
        let data = build_port_trade_frame(None, w, actor, parsed.after);
        check_keyframe_size(
            &data,
            MAX_TRADE_KEYFRAME_BYTES,
            "Port trade payload too large.",
        )?;
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(PORT_TRADE_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &Value,
    ) -> Result<OutputEvent, RichText> {
        const BAD_QUERY: &str = "Invalid port trade query.";
        let parsed: PortTradeParams =
            decode_closed_params(context.world(), actor, params, BAD_QUERY)?;
        let data = build_port_trade_frame(Some(context), context.world(), actor, parsed.after);
        check_keyframe_size(
            &data,
            MAX_TRADE_KEYFRAME_BYTES,
            "Port trade payload too large.",
        )?;
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(PORT_TRADE_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
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
                    if def.get("lock").and_then(Value::as_str) == Some("staff") {
                        return Err(format!(
                            "staff-locked entry leaked on `{}:{row_id}`",
                            entry.id
                        ));
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
                    {
                        return Err(format!("restricted room leaked on `{}:{row_id}`", entry.id));
                    }
                    if kind == "location"
                        && let Some(clusters) = def.get("clusters").and_then(Value::as_array)
                    {
                        for cluster in clusters {
                            if let Some(rocks) = cluster.get("rocks").and_then(Value::as_array) {
                                for rock in rocks {
                                    let Some(robj) = rock.as_object() else {
                                        return Err(format!(
                                            "non-object cluster rock on `{}:{row_id}`",
                                            entry.id
                                        ));
                                    };
                                    if robj.is_empty()
                                        || !robj.contains_key("letter")
                                        || !robj.contains_key("spectral")
                                    {
                                        return Err(format!(
                                            "cluster rock wiped or missing public chart keys on `{}:{row_id}`: {rock}",
                                            entry.id
                                        ));
                                    }
                                    if robj.contains_key("gross_t") {
                                        return Err(format!(
                                            "rock reserve `gross_t` leaked on `{}:{row_id}`",
                                            entry.id
                                        ));
                                    }
                                }
                            }
                        }
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
        for hidden in [
            "secret.slowdown",
            "exit.c14_c15",
            "exit.c05_c16",
            "room.c15_maintenance",
            "room.c16_authority",
            "item.chandlery_cage",
            "item.occator_manifold",
            "item.exchange_manifold",
            "staff only",
        ] {
            check!(!ceres.json.contains(hidden));
        }
        // CS-H1: staff-locked exits (`lock: "staff"`) and restricted rooms stripped from relays.
        for relay_nb in [
            "neighborhood.kirkwood_depot",
            "neighborhood.mercury_relay",
            "neighborhood.malapert_exchange",
            "neighborhood.phobos_exchange",
        ] {
            let Some(relay) = bundle.find(relay_nb) else {
                return crate::test_checks::failed(&format!("missing {relay_nb}"));
            };
            check!(!relay.json.contains("exit.relay"));
            check!(!relay.json.contains("room.relay"));
        }
        // CS-H1: NPCs stationed in restricted rooms stripped from projected cast packs.
        let Some(ceres_cast) = bundle.find("cast.ceres_freeport") else {
            return crate::test_checks::failed("missing cast.ceres_freeport");
        };
        check!(!ceres_cast.json.contains("npc.holm"));
        check!(ceres_cast.json.contains("npc.qureshi"));

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
        // CS-H2: cluster rocks preserve real public chart fields instead of wiping to `{}`.
        let lane_val: Value = serde_json::from_str(&lane.json)?;
        let Some(clusters) = lane_val["entries"]["kirkwood_depot"]["def"]["clusters"].as_array()
        else {
            return crate::test_checks::failed("missing kirkwood_depot clusters");
        };
        check!(!clusters.is_empty());
        let mut rock_count = 0usize;
        for cluster in clusters {
            let Some(rocks) = cluster["rocks"].as_array() else {
                return crate::test_checks::failed("missing rocks array on cluster");
            };
            for rock in rocks {
                rock_count += 1;
                check!(rock.get("letter").and_then(Value::as_str).is_some());
                check!(rock.get("alias").and_then(Value::as_str).is_some());
                check!(rock.get("spectral").and_then(Value::as_str).is_some());
                check!(rock.get("pos_m").and_then(Value::as_array).is_some());
                check!(rock.get("vel_mmps").and_then(Value::as_array).is_some());
                check!(rock.get("tenure").and_then(Value::as_str).is_some());
                check!(rock.get("gross_t").is_none());
            }
        }
        check!(rock_count >= 12);
        Ok(())
    }

    #[test]
    fn client_content_bundle_covers_every_content_neighborhood_region_unit_and_space_pack()
    -> TestResult {
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
            if let Some(region_id) = fname
                .strip_prefix("region.")
                .and_then(|s| s.strip_suffix(".json"))
                .map(|slug| format!("region.{slug}"))
            {
                let Some(pack) = bundle.find(&region_id) else {
                    return crate::test_checks::failed(&format!(
                        "region file `{fname}` missing from client_content_bundle"
                    ));
                };
                check_eq!(pack.category, "region");
                check!(!pack.json.contains("\"theme\""));
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

        // Missing packs (#55): space mounts/fits & industry hull recipes.
        for (alias, expected_id) in [
            ("space_mounts", "space_mounts"),
            ("tacsim.pack.space_mounts", "space_mounts"),
            ("space_fits", "space_fits"),
            ("tacsim.pack.space_fits", "space_fits"),
            ("hulls", "hulls"),
            ("tacsim.industry.hulls", "hulls"),
        ] {
            let Some(found) = bundle.find(alias) else {
                return crate::test_checks::failed(&format!("missing pack alias `{alias}`"));
            };
            check_eq!(found.id.as_ref(), expected_id);
        }
        Ok(())
    }

    #[test]
    fn client_content_query_enforces_closed_params_and_live_player_gate() -> TestResult {
        let mut tw = mush_testkit::TestWorld::new();
        let player = tw.spawn_player("PlayerViewer");
        let thing = tw
            .world_mut()
            .create(ObjKind::Thing, "NotAPlayer", ObjId(1), None)?;
        let going_player = tw.spawn_player("GoingPlayer");
        if let Some(obj) = tw.world_mut().get_mut(going_player) {
            obj.flags.insert(Flags::GOING);
        }
        let w = &tw.engine().world;
        let q = ClientContentQuery::new();

        // Valid empty / null / explicit manifest queries succeed.
        for valid in [json!({}), Value::Null, json!({"op": "manifest"})] {
            let Ok(OutputEvent::Panel { data, .. }) = q.query(w, player, &valid) else {
                return crate::test_checks::failed(&format!("expected ok for {valid}"));
            };
            check_eq!(data["op"], json!("manifest"));
        }

        // Malformed / unknown-field / amplification queries are refused closed (CS-M2).
        for bad in [
            json!({"op": "bogus"}),
            json!({"pack": "dray"}),
            json!({"id": "dray"}),
            json!({"op": "manifest", "id": "dray"}),
            json!({"op": "pack"}),
            json!({"op": "pack", "id": ""}),
            json!({"op": "pack", "id": "dray\n"}),
            json!({"op": "pack", "id": "a".repeat(200)}),
            json!({"op": "pack", "id": "dray", "extra": true}),
        ] {
            check!(q.query(w, player, &bad).is_err());
        }

        // Non-player and GOING actors are refused closed on both queries.
        check!(q.query(w, thing, &json!({})).is_err());
        check!(q.query(w, going_player, &json!({})).is_err());
        let trade_q = PortTradeQuery;
        check!(trade_q.query(w, thing, &json!({})).is_err());
        check!(trade_q.query(w, going_player, &json!({})).is_err());
        check!(trade_q.query(w, player, &json!({"bogus": 1})).is_err());
        check!(trade_q.query(w, player, &json!({})).is_ok());
        Ok(())
    }

    #[test]
    fn world_content_bundle_gates_cast_unapplied_packs_and_withdrawn_designs() -> TestResult {
        let mut tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_atlas::AtlasModule::default()),
            Box::new(crate::TacsimRuleset::default()),
        ]);
        let player = tw.spawn_player("PlayerViewer");
        // Stage an unapplied `CampaignSeed` (`ceres_phobos` lane) in `store.loaded` without
        // applying it — `world_content_bundle` must NOT expose it via `newest_loaded` (CS-H1).
        let lane_pack = crate::pack::load_pack(crate::definitions::content::LANE_JSON)?;
        {
            let Some(store) = tw.world_mut().module_state_mut::<TacsimStore>() else {
                return crate::test_checks::failed("TacsimStore missing from TacsimRuleset");
            };
            store.stage_pack(crate::LoadedPack {
                source: "<unapplied lane>".to_owned(),
                pack: lane_pack,
            })?;
            // Mark `tacsim.unit.dray` withdrawn in the design registry.
            store.designs.insert(
                (crate::definitions::content::DRAY.to_owned(), 1),
                crate::design::DesignRecord {
                    id: crate::definitions::content::DRAY.to_owned(),
                    version: 1,
                    pack: json!({}),
                    look: json!({}),
                    withdrawn: true,
                },
            );
        }

        // Non-staff sees neither `cast.ceres_freeport` (no accessible companion neighborhood),
        // nor the unapplied `ceres_phobos` lane pack, nor the withdrawn `dray` design!
        let player_bundle = world_content_bundle(tw.world(), player);
        check!(player_bundle.find("cast.ceres_freeport").is_none());
        check!(player_bundle.find("ceres_phobos").is_none());
        check!(player_bundle.find("dray").is_none());
        // Fingerprint changes when withdrawn state toggles (CS-M1).
        let fp1 = world_content_fingerprint(tw.world(), player);
        if let Some(st) = tw.world_mut().module_state_mut::<TacsimStore>()
            && let Some(rec) = st
                .designs
                .get_mut(&(crate::definitions::content::DRAY.to_owned(), 1))
        {
            rec.withdrawn = false;
        }
        let fp2 = world_content_fingerprint(tw.world(), player);
        check_ne!(fp1, fp2);

        // `hash_atlas_records` invalidates on `structures` & `neighborhoods` edits (CS-M1).
        let digest_records = |records: &mush_atlas::Records| -> [u8; 32] {
            let mut h = Sha256::new();
            hash_atlas_records(&mut h, tw.world(), records);
            h.finalize().into()
        };
        let mut records = mush_atlas::Records::default();
        let rec_fp0 = digest_records(&records);
        let pack_id = mush_core::CanonicalId::new("neighborhood.fp_probe")?;
        let st_key = mush_atlas::RecordKey::new(
            pack_id.clone(),
            mush_core::CanonicalId::new("structure.fp_hub")?,
        );
        let nb_key = mush_atlas::RecordKey::new(
            pack_id.clone(),
            mush_core::CanonicalId::new("neighborhood.fp_main")?,
        );
        records.structures.insert(
            st_key.clone(),
            mush_atlas::StructureRec {
                key: st_key,
                site: mush_atlas::RecordKey::new(
                    pack_id.clone(),
                    mush_core::CanonicalId::new("site.fp_hub")?,
                ),
                footprint: mush_atlas::Polygon2(vec![
                    mush_atlas::Point2(0.0, 0.0),
                    mush_atlas::Point2(10.0, 0.0),
                    mush_atlas::Point2(10.0, 10.0),
                    mush_atlas::Point2(0.0, 10.0),
                ]),
                floors: vec![],
                revision: 1,
            },
        );
        let rec_fp1 = digest_records(&records);
        check_ne!(rec_fp0, rec_fp1);
        records.neighborhoods.insert(
            nb_key.clone(),
            mush_atlas::NeighborhoodRec {
                key: nb_key,
                region: mush_atlas::RecordKey::new(
                    mush_core::CanonicalId::new("region.ceres")?,
                    mush_core::CanonicalId::new("region.ceres")?,
                ),
                locality: mush_atlas::RecordKey::new(
                    pack_id.clone(),
                    mush_core::CanonicalId::new("locality.fp")?,
                ),
                name: "FP Probe".to_owned(),
                setting: "station".to_owned(),
                representation: mush_atlas::record::Representation::Authored,
                footprint: mush_atlas::Polygon2(vec![]),
                room_budget: mush_atlas::record::Budget {
                    target: 10,
                    maximum: 10,
                },
                hydration: mush_atlas::record::HydrationPolicy::default(),
                zone: None,
                reviewers: vec![],
                tiles: vec![],
                resolution_cm: 100,
                theme: None,
                ambience: vec![],
                revision: 1,
            },
        );
        let rec_fp2 = digest_records(&records);
        check_ne!(rec_fp1, rec_fp2);
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
        use mush_atlas::{
            Access, AtlasState, Footprint, Point2, RecordKey, Rect2, RoomGeo, RoomKind, SurfaceRef,
        };
        use mush_core::CanonicalId;

        let fp_of = |w: &World, atlas: &AtlasState| -> [u8; 32] {
            let mut h = Sha256::new();
            hash_atlas_records(&mut h, w, &atlas.records);
            h.finalize().into()
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
        let mut tw_disabled = mush_testkit::TestWorld::with_modules(vec![Box::new(
            crate::TacsimRuleset::with_config(disabled_cfg),
        )]);
        let player = tw_disabled.spawn_player("DisabledStorePilot");
        let fp_disabled = world_content_fingerprint(&tw_disabled.engine().world, player);
        check_eq!(
            fp_disabled,
            world_content_fingerprint(&tw_disabled.engine().world, player)
        );
        let q = ClientContentQuery::new();
        let Ok(OutputEvent::Panel { data, .. }) = q.query(
            &tw_disabled.engine().world,
            player,
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
            player,
            &json!({"op": "pack", "id": "dray"}),
        )
        else {
            return crate::test_checks::failed("expected pack panel response");
        };
        check_eq!(pack_data["enabled"], json!(false));
        check_eq!(pack_data["found"], json!(false));
        Ok(())
    }

    #[test]
    fn reconcile_off_shelf_neighborhood_recovers_public_portals_and_strips_hidden_or_locked()
    -> TestResult {
        use mush_atlas::record::{
            Endpoint, Floor, PortalGeometry, PortalMode, PortalRec, TimePolicy, Visibility,
        };
        use mush_atlas::{
            Access, Footprint, Point2, Point3, Polygon2, RecordKey, Records, Rect2, RoomGeo,
            RoomKind, StructureRec, SurfaceRef,
        };
        use mush_core::CanonicalId;

        let mut tw = mush_testkit::TestWorld::new();
        let w = tw.world_mut();
        let r_berth = w.create(ObjKind::Room, "Off-Shelf Berth", ObjId(1), None)?;
        w.set_marker(r_berth, "port.dock", true)?;
        let r_hall = w.create(ObjKind::Room, "Off-Shelf Concourse", ObjId(1), None)?;
        let r_upper = w.create(ObjKind::Room, "Upper Observation", ObjId(1), None)?;
        let r_vault = w.create(ObjKind::Room, "Authority Vault", ObjId(1), None)?;
        let r_foreign = w.create(ObjKind::Room, "Neighboring Dock", ObjId(1), None)?;

        // Public bidirectional door: r_berth <-> r_hall
        let e_fwd = w.create(ObjKind::Exit, "Concourse <N>", ObjId(1), Some(r_berth))?;
        w.add_alias(e_fwd, "n")?;
        w.add_alias(e_fwd, "north")?;
        w.link_exit(e_fwd, Some(r_hall))?;
        let e_back = w.create(ObjKind::Exit, "Berth <S>", ObjId(1), Some(r_hall))?;
        w.add_alias(e_back, "s")?;
        w.link_exit(e_back, Some(r_berth))?;

        // Public inter-floor lift: r_hall <-> r_upper (Point geometry + dz_m)
        let e_up = w.create(ObjKind::Exit, "Lift Up <U>", ObjId(1), Some(r_hall))?;
        w.add_alias(e_up, "u")?;
        w.link_exit(e_up, Some(r_upper))?;
        let e_down = w.create(ObjKind::Exit, "Lift Down <D>", ObjId(1), Some(r_upper))?;
        w.add_alias(e_down, "d")?;
        w.link_exit(e_down, Some(r_hall))?;

        // Cross-pack public exit: r_hall <-> r_foreign
        let e_cross_fwd = w.create(ObjKind::Exit, "Bridge <X>", ObjId(1), Some(r_hall))?;
        w.add_alias(e_cross_fwd, "x")?;
        w.link_exit(e_cross_fwd, Some(r_foreign))?;

        // Hidden portal (PortalRec::visibility == Hidden)
        let e_hidden = w.create(ObjKind::Exit, "Service Hatch <H>", ObjId(1), Some(r_hall))?;
        w.link_exit(e_hidden, Some(r_upper))?;

        // DARK-flagged live exit
        let e_dark = w.create(ObjKind::Exit, "Dark Vent <V>", ObjId(1), Some(r_hall))?;
        w.link_exit(e_dark, Some(r_berth))?;
        w.set_flag(e_dark, Flags::DARK, true)?;

        // Staff-locked live exit
        let e_staff = w.create(ObjKind::Exit, "Staff Lift <SL>", ObjId(1), Some(r_hall))?;
        w.link_exit(e_staff, Some(r_upper))?;
        let staff_lock = mush_core::lock::parse_lock("flag^WIZARD|flag^ROYALTY", |_| None)
            .map_err(|e| format!("parse staff lock: {e}"))?;
        if let Some(o) = w.get_mut(e_staff) {
            o.locks.insert(mush_core::lock::LockType::Basic, staff_lock);
        }

        // Portal into restricted room
        let e_vault = w.create(ObjKind::Exit, "Vault Door <VD>", ObjId(1), Some(r_hall))?;
        w.link_exit(e_vault, Some(r_vault))?;

        let pack_id = CanonicalId::new("neighborhood.off_shelf_station")?;
        let foreign_pack = CanonicalId::new("neighborhood.foreign_annex")?;
        let nb_key = RecordKey::new(pack_id.clone(), CanonicalId::new("neighborhood.main")?);
        let st_key = RecordKey::new(pack_id.clone(), CanonicalId::new("structure.ring")?);

        let mut records = Records::default();
        records.structures.insert(
            st_key.clone(),
            StructureRec {
                key: st_key.clone(),
                site: RecordKey::new(pack_id.clone(), CanonicalId::new("site.ring")?),
                footprint: Polygon2(vec![
                    Point2(0.0, 0.0),
                    Point2(100.0, 0.0),
                    Point2(100.0, 100.0),
                    Point2(0.0, 100.0),
                ]),
                floors: vec![
                    Floor {
                        index: 0,
                        name: Some("Lower Deck".to_owned()),
                        z_offset_m: 0.0,
                        height_m: 4.5,
                        tile: None,
                    },
                    Floor {
                        index: 1,
                        name: Some("Upper Deck".to_owned()),
                        z_offset_m: 5.0,
                        height_m: 4.5,
                        tile: None,
                    },
                ],
                revision: 1,
            },
        );
        let add_room = |records: &mut Records,
                        room: ObjId,
                        pack: &CanonicalId,
                        entry: &str,
                        floor: i16,
                        rect: ((f64, f64), (f64, f64)),
                        access: Access|
         -> TestResult {
            records.roomgeo.insert(
                room,
                RoomGeo {
                    room,
                    key: RecordKey::new(pack.clone(), CanonicalId::new(entry)?),
                    neighborhood: nb_key.clone(),
                    site: None,
                    surface: SurfaceRef::Floor {
                        structure: st_key.clone(),
                        index: floor,
                    },
                    footprint: Footprint::Rect(Rect2(
                        Point2(rect.0.0, rect.0.1),
                        Point2(rect.1.0, rect.1.1),
                    )),
                    kind: RoomKind::Indoor,
                    access,
                    revision: 1,
                },
            );
            Ok(())
        };
        add_room(
            &mut records,
            r_berth,
            &pack_id,
            "room.o01_berth",
            0,
            ((0.0, 0.0), (20.0, 20.0)),
            Access::Public,
        )?;
        add_room(
            &mut records,
            r_hall,
            &pack_id,
            "room.o02_concourse",
            0,
            ((0.0, 20.0), (20.0, 40.0)),
            Access::Public,
        )?;
        add_room(
            &mut records,
            r_upper,
            &pack_id,
            "room.o03_upper",
            1,
            ((0.0, 20.0), (20.0, 40.0)),
            Access::Public,
        )?;
        add_room(
            &mut records,
            r_vault,
            &pack_id,
            "room.o04_vault",
            0,
            ((20.0, 20.0), (35.0, 40.0)),
            Access::Restricted,
        )?;
        add_room(
            &mut records,
            r_foreign,
            &foreign_pack,
            "room.f01_dock",
            0,
            ((0.0, 40.0), (20.0, 60.0)),
            Access::Public,
        )?;

        let add_portal = |records: &mut Records,
                          entry: &str,
                          exits: Vec<ObjId>,
                          from: ObjId,
                          to: ObjId,
                          geometry: PortalGeometry,
                          mode: PortalMode,
                          dz_m: f64,
                          visibility: Visibility|
         -> TestResult {
            let key = RecordKey::new(pack_id.clone(), CanonicalId::new(entry)?);
            records.portals.insert(
                key.clone(),
                PortalRec {
                    key,
                    exits,
                    from: Endpoint::Room(from),
                    to: Endpoint::Room(to),
                    geometry,
                    mode,
                    distance_m: 0,
                    dz_m,
                    time: TimePolicy::Instant,
                    visibility,
                    revision: 1,
                },
            );
            Ok(())
        };
        add_portal(
            &mut records,
            "exit.o01_o02",
            vec![e_fwd, e_back],
            r_berth,
            r_hall,
            PortalGeometry::Segment(Point3(0.0, 20.0, 0.0), Point3(20.0, 20.0, 0.0)),
            PortalMode::Door,
            0.0,
            Visibility::Public,
        )?;
        add_portal(
            &mut records,
            "exit.o02_o03",
            vec![e_up, e_down],
            r_hall,
            r_upper,
            PortalGeometry::Point(Point3(10.0, 30.0, 0.0)),
            PortalMode::Stair,
            5.0,
            Visibility::Public,
        )?;
        add_portal(
            &mut records,
            "exit.o02_foreign",
            vec![e_cross_fwd],
            r_hall,
            r_foreign,
            PortalGeometry::Point(Point3(10.0, 40.0, 0.0)),
            PortalMode::Open,
            0.0,
            Visibility::Public,
        )?;
        add_portal(
            &mut records,
            "exit.o02_hidden",
            vec![e_hidden],
            r_hall,
            r_upper,
            PortalGeometry::Point(Point3(2.0, 22.0, 0.0)),
            PortalMode::Hatch,
            5.0,
            Visibility::Hidden,
        )?;
        add_portal(
            &mut records,
            "exit.o02_dark",
            vec![e_dark],
            r_hall,
            r_berth,
            PortalGeometry::Point(Point3(18.0, 20.0, 0.0)),
            PortalMode::Hatch,
            0.0,
            Visibility::Public,
        )?;
        add_portal(
            &mut records,
            "exit.o02_staff",
            vec![e_staff],
            r_hall,
            r_upper,
            PortalGeometry::Point(Point3(18.0, 38.0, 0.0)),
            PortalMode::Door,
            5.0,
            Visibility::Public,
        )?;
        add_portal(
            &mut records,
            "exit.o02_vault",
            vec![e_vault],
            r_hall,
            r_vault,
            PortalGeometry::Segment(Point3(20.0, 20.0, 0.0), Point3(20.0, 40.0, 0.0)),
            PortalMode::Door,
            0.0,
            Visibility::Public,
        )?;

        // Start from the exact empty off-shelf skeleton that `world_content_bundle` builds.
        let mut doc = json!({
            "manifest": { "id": pack_id.as_str(), "version": 1, "kind": "neighborhood" },
            "entries": {}
        });
        reconcile_neighborhood_with_live_records(&tw.engine().world, &records, &pack_id, &mut doc);

        let Some(projected_str) = project_pack_value_for_client(doc) else {
            return crate::test_checks::failed("projection failed for off-shelf neighborhood");
        };
        let projected: Value = serde_json::from_str(&projected_str)?;
        let entries = projected["entries"]
            .as_object()
            .ok_or("missing entries object")?;

        // Public rooms + port.dock marker survive; restricted room is stripped.
        check!(entries.contains_key("room.o01_berth"));
        check_eq!(
            entries["room.o01_berth"]["def"]["markers"],
            json!(["port.dock"])
        );
        check!(entries.contains_key("room.o02_concourse"));
        check!(entries.contains_key("room.o03_upper"));
        check!(!entries.contains_key("room.o04_vault"));

        // Public walkable bidirectional door is recovered with endpoints, exit commands, and segment geometry.
        let door = &entries["exit.o01_o02"]["def"];
        check_eq!(door["from"], json!("room.o01_berth"));
        check_eq!(door["to"], json!("room.o02_concourse"));
        check_eq!(
            door["names"],
            json!(["Concourse <N>;n;north", "Berth <S>;s"])
        );
        check_eq!(door["portal"]["mode"], json!("door"));
        check_eq!(door["portal"]["visibility"], json!("public"));
        check_eq!(
            door["portal"]["at"],
            json!([[0.0, 20.0, 0.0], [20.0, 20.0, 0.0]])
        );

        // Public inter-floor lift preserves point `at` and `dz_m`.
        let lift = &entries["exit.o02_o03"]["def"];
        check_eq!(lift["from"], json!("room.o02_concourse"));
        check_eq!(lift["to"], json!("room.o03_upper"));
        check_eq!(lift["names"], json!(["Lift Up <U>;u", "Lift Down <D>;d"]));
        check_eq!(lift["portal"]["mode"], json!("stair"));
        check_eq!(lift["portal"]["dz_m"], json!(5.0));
        check_eq!(lift["portal"]["at"], json!([10.0, 30.0, 0.0]));

        // Cross-pack public exit preserves qualified endpoint key.
        let cross = &entries["exit.o02_foreign"]["def"];
        check_eq!(cross["from"], json!("room.o02_concourse"));
        check_eq!(
            cross["to"],
            json!("neighborhood.foreign_annex.room.f01_dock")
        );
        check_eq!(cross["names"], json!(["Bridge <X>;x"]));

        // Hidden, DARK, staff-locked, and restricted-destination portals are ALL stripped.
        for stripped in [
            "exit.o02_hidden",
            "exit.o02_dark",
            "exit.o02_staff",
            "exit.o02_vault",
        ] {
            check!(
                !entries.contains_key(stripped),
                "expected `{stripped}` to be stripped from projected off-shelf pack"
            );
        }

        // Fingerprint detects portal/exit mutations (e.g. exit rename or DARK flag).
        let digest = |w: &World, r: &Records| -> [u8; 32] {
            let mut h = Sha256::new();
            hash_atlas_records(&mut h, w, r);
            h.finalize().into()
        };
        let fp_before = digest(&tw.engine().world, &records);
        tw.world_mut().set_name(e_fwd, "Main Concourse <N>")?;
        let fp_after_rename = digest(&tw.engine().world, &records);
        check_ne!(fp_before, fp_after_rename);
        tw.world_mut().set_flag(e_fwd, Flags::DARK, true)?;
        let fp_after_dark = digest(&tw.engine().world, &records);
        check_ne!(fp_after_rename, fp_after_dark);

        Ok(())
    }

    #[test]
    fn client_content_query_reuses_warm_cache_in_o1_and_invalidates_on_mutations() -> TestResult {
        use std::sync::atomic::Ordering;

        let mut tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_atlas::AtlasModule::default()),
            Box::new(crate::TacsimRuleset::default()),
        ]);
        let room_a = tw
            .world_mut()
            .create(ObjKind::Room, "Dock A", ObjId(1), None)?;
        let room_b = tw
            .world_mut()
            .create(ObjKind::Room, "Dock B", ObjId(1), None)?;
        let player = tw.spawn_player("CachePilot");
        tw.world_mut().move_to(player, room_a)?;

        let q = ClientContentQuery::new();

        // Cold manifest + 20 warm pack downloads must scan and build ONLY ONCE.
        let b0 = q.bundle_for(tw.world(), player);
        check_eq!(q.fingerprint_scans.load(Ordering::Relaxed), 1);
        check_eq!(q.bundle_builds.load(Ordering::Relaxed), 1);
        for _ in 0..20 {
            let bn = q.bundle_for(tw.world(), player);
            check!(Arc::ptr_eq(&b0, &bn));
        }
        check_eq!(
            q.fingerprint_scans.load(Ordering::Relaxed),
            1,
            "warm pack downloads must hit O(1) revision cache without rescanning atlas"
        );
        check_eq!(q.bundle_builds.load(Ordering::Relaxed), 1);

        // Unrelated runtime activity (actor movement & world clock ticks) must NOT trigger a rescan.
        tw.world_mut().move_to(player, room_b)?;
        tw.world_mut().set_now(123_456);
        let b_after_move = q.bundle_for(tw.world(), player);
        check!(Arc::ptr_eq(&b0, &b_after_move));
        check_eq!(q.fingerprint_scans.load(Ordering::Relaxed), 1);
        check_eq!(q.bundle_builds.load(Ordering::Relaxed), 1);

        // Unrelated TacsimStore write bumps tacsim_rev -> Level-2 fingerprint check runs ONCE
        // and reuses the cached Arc without rebuilding, then returns to O(1) on subsequent hits.
        let _ = tw.world_mut().module_state_mut::<TacsimStore>();
        let b_after_noop_store = q.bundle_for(tw.world(), player);
        check!(Arc::ptr_eq(&b0, &b_after_noop_store));
        check_eq!(q.fingerprint_scans.load(Ordering::Relaxed), 2);
        check_eq!(q.bundle_builds.load(Ordering::Relaxed), 1);
        let _ = q.bundle_for(tw.world(), player);
        check_eq!(q.fingerprint_scans.load(Ordering::Relaxed), 2);

        // Content-relevant TacsimStore mutation (withdrawing `dray`) invalidates and rebuilds.
        {
            let Some(store) = tw.world_mut().module_state_mut::<TacsimStore>() else {
                return crate::test_checks::failed("TacsimStore missing");
            };
            store.designs.insert(
                (crate::definitions::content::DRAY.to_owned(), 1),
                crate::design::DesignRecord {
                    id: crate::definitions::content::DRAY.to_owned(),
                    version: 1,
                    pack: json!({}),
                    look: json!({}),
                    withdrawn: true,
                },
            );
        }
        let b_withdrawn = q.bundle_for(tw.world(), player);
        check!(!Arc::ptr_eq(&b0, &b_withdrawn));
        check!(b_withdrawn.find("dray").is_none());
        check_eq!(q.fingerprint_scans.load(Ordering::Relaxed), 3);
        check_eq!(q.bundle_builds.load(Ordering::Relaxed), 2);
        // Subsequent warm queries stay O(1).
        let b_withdrawn_2 = q.bundle_for(tw.world(), player);
        check!(Arc::ptr_eq(&b_withdrawn, &b_withdrawn_2));
        check_eq!(q.fingerprint_scans.load(Ordering::Relaxed), 3);

        // Room metadata mutation (`set_name`) bumps `obj_meta_rev` and triggers fingerprint check.
        tw.world_mut().set_name(room_a, "Dock Alpha")?;
        let _ = q.bundle_for(tw.world(), player);
        check_eq!(q.fingerprint_scans.load(Ordering::Relaxed), 4);

        // A freshly created World instance cannot falsely match the prior World's rev tuple.
        let mut tw2 = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_atlas::AtlasModule::default()),
            Box::new(crate::TacsimRuleset::default()),
        ]);
        let player2 = tw2.spawn_player("FreshWorldPilot");
        let b_fresh = q.bundle_for(tw2.world(), player2);
        check!(b_fresh.find("dray").is_some());
        check_eq!(q.bundle_builds.load(Ordering::Relaxed), 3);

        Ok(())
    }
}
