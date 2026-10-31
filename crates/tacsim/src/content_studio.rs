//! **The staff content studio's server seam** (doc 34 cut CS.0a, D-34-2,
//! D-34-5, D-34-6, D-34-7, D-34-8, D-34-9).
//!
//! Answers [`mush_proto::ContentStudioRequest`] (`service: "content_studio"`):
//! - `Catalog` — live reference catalog of all 11 corridor ports, station/site
//!   rooms with authored and live `atlas:detail:<kw>` keys, NPCs with code-bound
//!   flags (`port/relationships.rs`, `lane/ceres_phobos.json` vendor signatories),
//!   lore pools, 12 goods, registered interface providers and command refs, and
//!   shipped/published pack summaries.
//! - `Read` — loads the full JSON document for a requested pack id or standalone
//!   table (`market/goods.json`, `resources.json`, `tools/*.json`).
//! - `Check` — reconciles `manifest.requires`, computes the canonical SHA-256
//!   `manifest.digest` under [`crate::pack::PACK_DIGEST_DOMAIN`] (or standalone
//!   table digest), runs strict schema/bound validators, and when `live_check`
//!   is true runs [`mush_content::check_pack`] against the attached live world.
//! - `Publish` — signs and applies a `campaign_seed` pack live to the running
//!   world via [`mush_content::apply_pack`] + [`mush_atlas::stage_pack`] (with
//!   optional live NPC relocation per D-34-9), or persists a `rules_definition`
//!   pack into the studio registry for next boot.
//! - `Assist` — `BUILDER_LLM`-gated co-authoring (`LlmFeature::ContentAssist`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{InvokeHandler, ModError, Registrar};
use mush_core::{AttrFlags, CanonicalId, ConnId, Ctx, Flags, GOD, ObjKind, Powers, perm};
use mush_proto::{ContentStudioRequest, ContentStudioResponse, InvokeRequest, InvokeResult};
use serde_json::{Value, json};

use crate::definitions::Catalog;
use crate::definitions::content::{
    AUTOCANNON_30_JSON, BALL_762_JSON, BATTERY_MK1_JSON, CANNON_105_JSON, CAST_SOURCES, CORES_JSON,
    DRILL_CORE_MK1_JSON, ECCM_MK1_JSON, ECM_MK1_JSON, HE_105_JSON, LANE_JSON, LANE_PACK,
    LASER_CHARGE_JSON, LASER_MK1_JSON, MG_COAX_JSON, MISSILE_LR_JSON, MISSILE_ROUND_JSON,
    RESOLUTION_JSON, RESOURCES_JSON, SCANNER_MK1_JSON, SITE_SOURCES, STATION_SOURCES,
    THERMAL_MK1_JSON, VISUAL_MK1_JSON, fielded_sources,
};
use crate::definitions::goods::{GOODS_JSON, GoodsTable, goods_digest, goods_table};
use crate::definitions::resources::ResourceTable;
use crate::definitions::tools::{ToolDef, tool_digest};
use crate::pack::PackKind;
use crate::{LoadedPack, TacsimConfig, TacsimStore, verify_sources_with};

/// The invoke service name in [`mush_core::module::MODULE_INVOKE_FAMILIES`].
pub const SERVICE: &str = "content_studio";

/// Register the `content_studio` invoke handler on [`crate::TacsimRuleset`].
///
/// # Errors
///
/// [`ModError`] if the service is already registered.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.invoke_handler(SERVICE, Arc::new(ContentStudioService))
}

struct ContentStudioService;

impl InvokeHandler for ContentStudioService {
    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        _conn: ConnId,
        _id: u64,
        request: &InvokeRequest,
    ) -> InvokeResult {
        dispatch(ctx, request).bounded()
    }
}

fn dispatch(ctx: &mut Ctx<'_>, request: &InvokeRequest) -> InvokeResult {
    let InvokeRequest::ContentStudio(request) = request else {
        return error("wrong_service", "That request is not for content_studio.");
    };
    if !crate::commands::is_staff(ctx.w, ctx.actor) {
        return error("not_staff", "Permission denied.");
    }
    if let Err(detail) = request.validate() {
        return error("invalid", detail);
    }
    match request {
        ContentStudioRequest::Catalog {} => build_catalog(ctx),
        ContentStudioRequest::Read { id } => read_document(ctx, id),
        ContentStudioRequest::Check {
            id,
            content,
            live_check,
        } => check_document(ctx, id, content.clone(), *live_check),
        ContentStudioRequest::Publish {
            id,
            content,
            relocate_npcs,
        } => publish_document(ctx, id, content.clone(), relocate_npcs),
        ContentStudioRequest::Assist {
            family,
            scope,
            brief,
            context,
        } => assist_document(ctx, family, scope, brief, context),
    }
}

fn error(code: &str, message: impl Into<String>) -> InvokeResult {
    InvokeResult::ContentStudio(ContentStudioResponse::Error {
        code: code.to_string(),
        message: message.into(),
    })
}

/// Metadata describing one shipped or studio-editable document.
struct DocSpec {
    id: &'static str,
    repo_path: &'static str,
    family: &'static str,
    bytes: &'static str,
}

fn shipped_docs() -> Vec<DocSpec> {
    let mut docs = vec![
        // Crew & Passengers
        DocSpec {
            id: crate::detachment::crew::content::PACK,
            repo_path: "crew/ceres.json",
            family: "npc",
            bytes: crate::detachment::crew::content::JSON,
        },
        DocSpec {
            id: crate::quest::packs::PASSENGER_PACK,
            repo_path: "passengers/ceres.json",
            family: "npc",
            bytes: crate::quest::packs::PASSENGER_JSON,
        },
        DocSpec {
            id: crate::port::passenger::content::PACK,
            repo_path: "passengers/stickney.json",
            family: "npc",
            bytes: crate::port::passenger::content::JSON,
        },
        // Quests
        DocSpec {
            id: crate::quest::packs::PACK,
            repo_path: "quests/first_steps.json",
            family: "quest",
            bytes: crate::quest::packs::JSON,
        },
        DocSpec {
            id: crate::quest::packs::LEILA_PACK,
            repo_path: "quests/leila_followup.json",
            family: "quest",
            bytes: crate::quest::packs::LEILA_JSON,
        },
        DocSpec {
            id: crate::quest::packs::CONTACT_PACK,
            repo_path: "quests/port_connections.json",
            family: "quest",
            bytes: crate::quest::packs::CONTACT_JSON,
        },
        DocSpec {
            id: crate::quest::packs::SAFE_PASSAGE_PACK,
            repo_path: "quests/safe_passage.json",
            family: "quest",
            bytes: crate::quest::packs::SAFE_PASSAGE_JSON,
        },
        DocSpec {
            id: crate::quest::packs::SURVEY_WORK_PACK,
            repo_path: "quests/survey_work.json",
            family: "quest",
            bytes: crate::quest::packs::SURVEY_WORK_JSON,
        },
        // Components & Rules Definitions
        DocSpec {
            id: "tacsim.pack.cannon_105",
            repo_path: "weapons/cannon_105.json",
            family: "component",
            bytes: CANNON_105_JSON,
        },
        DocSpec {
            id: "tacsim.pack.mg_coax",
            repo_path: "weapons/mg_coax.json",
            family: "component",
            bytes: MG_COAX_JSON,
        },
        DocSpec {
            id: "tacsim.pack.laser_mk1",
            repo_path: "weapons/laser_mk1.json",
            family: "component",
            bytes: LASER_MK1_JSON,
        },
        DocSpec {
            id: "tacsim.pack.autocannon_30",
            repo_path: "weapons/autocannon_30.json",
            family: "component",
            bytes: AUTOCANNON_30_JSON,
        },
        DocSpec {
            id: "tacsim.pack.missile_lr",
            repo_path: "weapons/missile_lr.json",
            family: "component",
            bytes: MISSILE_LR_JSON,
        },
        DocSpec {
            id: "tacsim.pack.he_105",
            repo_path: "ammunition/he_105.json",
            family: "component",
            bytes: HE_105_JSON,
        },
        DocSpec {
            id: "tacsim.pack.ball_762",
            repo_path: "ammunition/ball_762.json",
            family: "component",
            bytes: BALL_762_JSON,
        },
        DocSpec {
            id: "tacsim.pack.laser_charge",
            repo_path: "ammunition/laser_charge.json",
            family: "component",
            bytes: LASER_CHARGE_JSON,
        },
        DocSpec {
            id: "tacsim.pack.ap_30",
            repo_path: "ammunition/ap_30.json",
            family: "component",
            bytes: crate::definitions::content::AP_30_JSON,
        },
        DocSpec {
            id: "tacsim.pack.missile_round",
            repo_path: "ammunition/missile_round.json",
            family: "component",
            bytes: MISSILE_ROUND_JSON,
        },
        DocSpec {
            id: "tacsim.pack.visual_mk1",
            repo_path: "sensors/visual_mk1.json",
            family: "component",
            bytes: VISUAL_MK1_JSON,
        },
        DocSpec {
            id: "tacsim.pack.thermal_mk1",
            repo_path: "sensors/thermal_mk1.json",
            family: "component",
            bytes: THERMAL_MK1_JSON,
        },
        DocSpec {
            id: "tacsim.pack.ecm_mk1",
            repo_path: "sensors/ecm_mk1.json",
            family: "component",
            bytes: ECM_MK1_JSON,
        },
        DocSpec {
            id: "tacsim.pack.eccm_mk1",
            repo_path: "sensors/eccm_mk1.json",
            family: "component",
            bytes: ECCM_MK1_JSON,
        },
        DocSpec {
            id: "tacsim.pack.cores",
            repo_path: "cores/cores.json",
            family: "component",
            bytes: CORES_JSON,
        },
        DocSpec {
            id: crate::hardware::PACK,
            repo_path: "hardware/governed_hardware.json",
            family: "component",
            bytes: include_str!("../content/hardware/governed_hardware.json"),
        },
        DocSpec {
            id: "tacsim.pack.resolution",
            repo_path: "resolution.json",
            family: "component",
            bytes: RESOLUTION_JSON,
        },
        // Standalone typed tables
        DocSpec {
            id: "market/goods.json",
            repo_path: "market/goods.json",
            family: "component",
            bytes: GOODS_JSON,
        },
        DocSpec {
            id: "resources.json",
            repo_path: "resources.json",
            family: "component",
            bytes: RESOURCES_JSON,
        },
        DocSpec {
            id: "tools/drill_core_mk1.json",
            repo_path: "tools/drill_core_mk1.json",
            family: "component",
            bytes: DRILL_CORE_MK1_JSON,
        },
        DocSpec {
            id: "tools/scanner_mk1.json",
            repo_path: "tools/scanner_mk1.json",
            family: "component",
            bytes: SCANNER_MK1_JSON,
        },
        DocSpec {
            id: "tools/battery_mk1.json",
            repo_path: "tools/battery_mk1.json",
            family: "component",
            bytes: BATTERY_MK1_JSON,
        },
        // Corridor & Relays
        DocSpec {
            id: LANE_PACK,
            repo_path: "lane/ceres_phobos.json",
            family: "corridor",
            bytes: LANE_JSON,
        },
        DocSpec {
            id: "tacsim.relay.luna_ceres",
            repo_path: "relays/luna_ceres.json",
            family: "corridor",
            bytes: crate::relay_content::JSON,
        },
        DocSpec {
            id: "tacsim.relay.manufacture",
            repo_path: "relays/manufacture-v1.json",
            family: "corridor",
            bytes: include_str!("../content/relays/manufacture-v1.json"),
        },
    ];
    // Station cast packs (52 station NPCs and 20 lore pools split from `atlas/neighborhood.*.json`, D-34-3).
    for (pack_id, bytes) in CAST_SOURCES {
        let repo_path = match pack_id {
            "tacsim.cast.ceres_freeport" => "cast/ceres_freeport.json",
            "tacsim.cast.stickney_yards" => "cast/stickney_yards.json",
            "tacsim.cast.aitken_highport" => "cast/aitken_highport.json",
            "tacsim.cast.voltaire_anchorage" => "cast/voltaire_anchorage.json",
            "tacsim.cast.ligeia_terminal" => "cast/ligeia_terminal.json",
            _ => "cast/station.json",
        };
        docs.push(DocSpec {
            id: pack_id,
            repo_path,
            family: "npc",
            bytes,
        });
    }
    docs
}

fn infer_repo_path(id: &str) -> String {
    if let Some(doc) = shipped_docs()
        .into_iter()
        .find(|d| d.id == id || d.repo_path == id)
    {
        return doc.repo_path.to_string();
    }
    if let Some(slug) = id.strip_prefix("tacsim.cast.") {
        return format!("cast/{slug}.json");
    }
    if let Some(slug) = id.strip_prefix("tacsim.crew.") {
        return format!("crew/{slug}.json");
    }
    if let Some(slug) = id.strip_prefix("tacsim.passengers.") {
        return format!("passengers/{slug}.json");
    }
    if let Some(slug) = id.strip_prefix("tacsim.quests.") {
        return format!("quests/{slug}.json");
    }
    if let Some(slug) = id.strip_prefix("tacsim.pack.") {
        return format!("packs/{slug}.json");
    }
    if id.ends_with(".json") {
        return id.to_string();
    }
    format!("{id}.json")
}

/// Code-bound NPC bindings from `port/relationships.rs` and `lane/ceres_phobos.json`.
fn code_bound_npcs() -> BTreeMap<(String, String), Vec<String>> {
    let mut bound: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (pack_id, npc_id, reason) in [
        (
            "neighborhood.ceres_freeport",
            "npc.oduya",
            "crates/tacsim/src/port/relationships.rs (Freeport Harbourmaster desk)",
        ),
        (
            "tacsim.cast.ceres_freeport",
            "npc.oduya",
            "crates/tacsim/src/port/relationships.rs (Freeport Harbourmaster desk)",
        ),
        (
            "neighborhood.ceres_freeport",
            "npc.qureshi",
            "crates/tacsim/src/port/relationships.rs (Belt Transit Cooperative desk)",
        ),
        (
            "tacsim.cast.ceres_freeport",
            "npc.qureshi",
            "crates/tacsim/src/port/relationships.rs (Belt Transit Cooperative desk)",
        ),
    ] {
        bound
            .entry((pack_id.to_string(), npc_id.to_string()))
            .or_default()
            .push(reason.to_string());
    }
    if let Ok(lane_val) = serde_json::from_str::<Value>(LANE_JSON)
        && let Some(entries) = lane_val.get("entries").and_then(Value::as_object)
    {
        for (port_id, entry) in entries {
            if entry.get("kind").and_then(Value::as_str) != Some("port") {
                continue;
            }
            let Some(def) = entry.get("def") else {
                continue;
            };
            let area = def
                .get("area")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(vendors) = def.get("vendors").and_then(Value::as_array) {
                for vendor in vendors {
                    let v_id = vendor.get("id").and_then(Value::as_str).unwrap_or("vendor");
                    if let Some(sig) = vendor.get("signatory").and_then(Value::as_str) {
                        let npc_id = sig
                            .strip_prefix(&format!("{area}."))
                            .unwrap_or(sig)
                            .to_string();
                        let reason =
                            format!("lane/ceres_phobos.json ({port_id} vendor `{v_id}` signatory)");
                        bound
                            .entry((area.clone(), npc_id.clone()))
                            .or_default()
                            .push(reason.clone());
                        if let Some(station) = area.strip_prefix("neighborhood.") {
                            bound
                                .entry((format!("tacsim.cast.{station}"), npc_id))
                                .or_default()
                                .push(reason);
                        }
                    }
                }
            }
        }
    }
    bound
}

fn build_catalog(ctx: &Ctx<'_>) -> InvokeResult {
    let content_state = ctx.w.module_state::<mush_content::ContentState>();
    let atlas_state = ctx.w.module_state::<mush_atlas::AtlasState>();

    // 1. Ports from lane pack
    let mut ports = Vec::new();
    if let Ok(lane_val) = serde_json::from_str::<Value>(LANE_JSON)
        && let Some(entries) = lane_val.get("entries").and_then(Value::as_object)
    {
        for (id, entry) in entries {
            if entry.get("kind").and_then(Value::as_str) != Some("port") {
                continue;
            }
            if let Some(def) = entry.get("def") {
                let org_ids: Vec<String> = def
                    .get("orgs")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|o| o.get("id").and_then(Value::as_str).map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let vendor_ids: Vec<String> = def
                    .get("vendors")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.get("id").and_then(Value::as_str).map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                ports.push(json!({
                    "id": id,
                    "venue": def.get("venue").and_then(Value::as_str).unwrap_or(id),
                    "location": def.get("location").and_then(Value::as_str).unwrap_or(""),
                    "area": def.get("area").and_then(Value::as_str).unwrap_or(""),
                    "rooms": def.get("rooms").cloned().unwrap_or_else(|| json!({})),
                    "orgs": org_ids,
                    "vendors": vendor_ids,
                }));
            }
        }
    }

    // 2. Stations & Sites (rooms + authored details + live details)
    let mut stations = Vec::new();
    for (pack_id, bytes) in STATION_SOURCES.iter().chain(SITE_SOURCES.iter()) {
        if !pack_id.starts_with("neighborhood.") {
            continue;
        }
        let Ok(doc) = serde_json::from_str::<Value>(bytes) else {
            continue;
        };
        let version = doc
            .get("manifest")
            .and_then(|m| m.get("version"))
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let latched_latch = CanonicalId::new(pack_id)
            .ok()
            .and_then(|cid| content_state.and_then(|cs| cs.latches.applied(&cid)));
        let mut rooms = Vec::new();
        if let Some(entries) = doc.get("entries").and_then(Value::as_object) {
            for (entry_id, entry) in entries {
                if entry.get("kind").and_then(Value::as_str) != Some("room") {
                    continue;
                }
                let Some(def) = entry.get("def") else {
                    continue;
                };
                let mut detail_keys: BTreeSet<String> = def
                    .get("details")
                    .and_then(Value::as_object)
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                // Also inspect live room object if latched
                if let Some(latch) = latched_latch
                    && let Ok(room_cid) = CanonicalId::new(entry_id)
                    && let Some(room_obj) = latch
                        .entries()
                        .get(&room_cid)
                        .and_then(mush_content::AppliedEntry::object)
                    && let Some(obj) = ctx.w.get(room_obj)
                {
                    detail_keys.clear();
                    for (attr_name, attr) in obj.attrs.iter() {
                        if let Some(kw) = attr_name.as_str().strip_prefix("atlas:detail:")
                            && attr.owner == GOD
                            && !attr.flags.intersects(
                                AttrFlags::WIZARD_ONLY | AttrFlags::SELF_ONLY | AttrFlags::HIDDEN,
                            )
                            && attr.as_text().is_some_and(|t| !t.trim().is_empty())
                        {
                            detail_keys.insert(kw.to_string());
                        }
                    }
                }
                rooms.push(json!({
                    "id": entry_id,
                    "name": def.get("name").and_then(Value::as_str).unwrap_or(entry_id),
                    "zone": def.get("zone").and_then(Value::as_str).unwrap_or(""),
                    "tags": def.get("tags").cloned().unwrap_or_else(|| json!([])),
                    "details": detail_keys.into_iter().collect::<Vec<_>>(),
                }));
            }
        }
        stations.push(json!({
            "pack_id": pack_id,
            "version": version,
            "latched_version": latched_latch.map(mush_content::Applied::version),
            "rooms": rooms,
        }));
    }

    // 3. Collect all packs (shipped + shelf overrides) to index NPCs, lore pools, quests
    let code_bounds = code_bound_npcs();
    let mut npcs = Vec::new();
    let mut lore_pools = Vec::new();
    let mut quest_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut pack_summaries = Vec::new();

    // First pass over all documents to count quests per NPC
    let docs = shipped_docs();
    for spec in &docs {
        let doc_val = shelf_or_shipped_value(atlas_state, spec);
        if let Some(entries) = doc_val.get("entries").and_then(Value::as_object) {
            for entry in entries.values() {
                if entry.get("kind").and_then(Value::as_str) == Some("quest")
                    && let Some(def) = entry.get("def")
                    && let Some(npc) = def.get("npc").and_then(Value::as_str)
                {
                    let npc_pack = def
                        .get("npc_pack")
                        .and_then(Value::as_str)
                        .unwrap_or(spec.id)
                        .to_string();
                    *quest_counts.entry((npc_pack, npc.to_string())).or_insert(0) += 1;
                }
            }
        }
    }

    for spec in &docs {
        let doc_val = shelf_or_shipped_value(atlas_state, spec);
        let version = doc_val
            .get("manifest")
            .and_then(|m| m.get("version"))
            .and_then(Value::as_u64)
            .or_else(|| doc_val.get("version").and_then(Value::as_u64))
            .unwrap_or(1);
        let kind = doc_val
            .get("manifest")
            .and_then(|m| m.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("standalone_table");
        let entry_count = doc_val
            .get("entries")
            .and_then(Value::as_object)
            .map(serde_json::Map::len)
            .or_else(|| doc_val.get("goods").and_then(Value::as_array).map(Vec::len))
            .or_else(|| {
                doc_val
                    .get("resources")
                    .and_then(Value::as_array)
                    .map(Vec::len)
            })
            .unwrap_or(1);
        let latch = CanonicalId::new(spec.id)
            .ok()
            .and_then(|cid| content_state.and_then(|cs| cs.latches.applied(&cid)));

        pack_summaries.push(json!({
            "id": spec.id,
            "repo_path": spec.repo_path,
            "family": spec.family,
            "kind": kind,
            "version": version,
            "entry_count": entry_count,
            "latched_version": latch.map(mush_content::Applied::version),
            "retired": latch.is_some_and(mush_content::Applied::is_retired),
        }));

        let Some(entries) = doc_val.get("entries").and_then(Value::as_object) else {
            continue;
        };

        // Map crew offers and passenger roles inside this pack
        let mut crew_npcs = BTreeSet::new();
        let mut passenger_npcs = BTreeSet::new();
        for entry in entries.values() {
            match entry.get("kind").and_then(Value::as_str) {
                Some("crew_offer") => {
                    if let Some(npc) = entry
                        .get("def")
                        .and_then(|d| d.get("npc"))
                        .and_then(Value::as_str)
                    {
                        crew_npcs.insert(npc.to_string());
                    }
                }
                Some("passenger") => {
                    if let Some(npc) = entry
                        .get("def")
                        .and_then(|d| d.get("npc"))
                        .and_then(Value::as_str)
                    {
                        passenger_npcs.insert(npc.to_string());
                    }
                }
                _ => {}
            }
        }

        for (entry_id, entry) in entries {
            match entry.get("kind").and_then(Value::as_str) {
                Some("npc") => {
                    let Some(def) = entry.get("def") else {
                        continue;
                    };
                    let role = if crew_npcs.contains(entry_id) {
                        "crew"
                    } else if passenger_npcs.contains(entry_id) {
                        "passenger"
                    } else {
                        "station"
                    };
                    let room_pack = def
                        .get("room_pack")
                        .and_then(Value::as_str)
                        .unwrap_or(spec.id);
                    let bindings = code_bounds
                        .get(&(spec.id.to_string(), entry_id.clone()))
                        .cloned()
                        .unwrap_or_default();
                    let q_count = quest_counts
                        .get(&(spec.id.to_string(), entry_id.clone()))
                        .copied()
                        .unwrap_or(0);
                    let latched_entry = latch.and_then(|l| {
                        CanonicalId::new(entry_id)
                            .ok()
                            .and_then(|cid| l.entries().get(&cid))
                    });
                    npcs.push(json!({
                        "pack_id": spec.id,
                        "entry_id": entry_id,
                        "name": def.get("name").and_then(Value::as_str).unwrap_or(entry_id),
                        "room_pack": room_pack,
                        "room": def.get("room").and_then(Value::as_str).unwrap_or(""),
                        "tier": def.get("tier").and_then(Value::as_str).unwrap_or("minor"),
                        "role": role,
                        "interfaces": def.get("profile").and_then(|p| p.get("interfaces")).cloned().unwrap_or_else(|| json!([])),
                        "command_refs": def.get("profile").and_then(|p| p.get("command_refs")).cloned().unwrap_or_else(|| json!([])),
                        "tags": def.get("tags").cloned().unwrap_or_else(|| json!([])),
                        "quest_count": q_count,
                        "code_bound": !bindings.is_empty(),
                        "code_bindings": bindings,
                        "latched": latched_entry.is_some(),
                    }));
                }
                Some("lore") => {
                    if let Some(def) = entry.get("def") {
                        let count = def
                            .get("entries")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len);
                        lore_pools.push(json!({
                            "pack_id": spec.id,
                            "entry_id": entry_id,
                            "tag": def.get("tag").and_then(Value::as_str).unwrap_or(""),
                            "entry_count": count,
                        }));
                    }
                }
                _ => {}
            }
        }
    }

    // 4. Goods summary
    let goods: Vec<Value> = goods_table()
        .map(|t| {
            t.goods
                .into_iter()
                .map(|g| {
                    json!({
                        "id": g.id.as_str(),
                        "label": g.label,
                        "family": g.family.as_ref().map(CanonicalId::as_str),
                        "class": g.class.as_str(),
                        "ledger_unit": g.ledger_unit,
                        "quote_unit": g.quote_unit,
                        "stowage": g.stowage.as_str(),
                        "measures": g.measures,
                        "reference_major": g.reference_major,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let interfaces = vec!["hire", "passenger", "quest", "help", "coop"];
    let command_refs = vec![
        "+crew", "+passage", "+ship", "+quest", "+market", "+coop", "+survey", "+tv", "+chart",
        "+credit", "+sortie", "look",
    ];

    InvokeResult::ContentStudio(ContentStudioResponse::Catalog {
        catalog: json!({
            "ports": ports,
            "stations": stations,
            "npcs": npcs,
            "lore_pools": lore_pools,
            "goods": goods,
            "interfaces": interfaces,
            "command_refs": command_refs,
            "packs": pack_summaries,
        }),
    })
}

fn shelf_or_shipped_value(atlas_state: Option<&mush_atlas::AtlasState>, spec: &DocSpec) -> Value {
    if let Some(atlas) = atlas_state
        && let Ok(cid) = CanonicalId::new(spec.id)
        && let Some(pack) = atlas.shelf.get(&cid)
        && let Ok(val) = serde_json::to_value(pack)
    {
        return val;
    }
    serde_json::from_str(spec.bytes).unwrap_or_else(|_| json!({}))
}

fn read_document(ctx: &Ctx<'_>, id: &str) -> InvokeResult {
    let content_state = ctx.w.module_state::<mush_content::ContentState>();
    let atlas_state = ctx.w.module_state::<mush_atlas::AtlasState>();

    // Check shelf first for live-published packs
    if let Some(atlas) = atlas_state
        && let Ok(cid) = CanonicalId::new(id)
        && let Some(pack) = atlas.shelf.get(&cid)
        && let Ok(content) = serde_json::to_value(pack)
    {
        let latch = content_state.and_then(|cs| cs.latches.applied(&cid));
        return InvokeResult::ContentStudio(ContentStudioResponse::Document {
            id: id.to_string(),
            repo_path: infer_repo_path(id),
            latched_version: latch.map(mush_content::Applied::version),
            retired: latch.is_some_and(mush_content::Applied::is_retired),
            content,
        });
    }

    // Check shipped documents by pack id or repo_path
    if let Some(spec) = shipped_docs()
        .into_iter()
        .find(|d| d.id == id || d.repo_path == id)
    {
        let Ok(content) = serde_json::from_str::<Value>(spec.bytes) else {
            return error(
                "corrupt_shipped",
                "Shipped content document failed to parse.",
            );
        };
        let latch = CanonicalId::new(spec.id)
            .ok()
            .and_then(|cid| content_state.and_then(|cs| cs.latches.applied(&cid)));
        return InvokeResult::ContentStudio(ContentStudioResponse::Document {
            id: spec.id.to_string(),
            repo_path: spec.repo_path.to_string(),
            latched_version: latch.map(mush_content::Applied::version),
            retired: latch.is_some_and(mush_content::Applied::is_retired),
            content,
        });
    }

    error(
        "not_found",
        format!("Unknown content studio document `{id}`."),
    )
}

struct CheckedDocument {
    pack: Option<mush_content::Pack>,
    signed_doc: Value,
    digest: Option<String>,
    cascade: Vec<Value>,
}

/// Core validation + signing logic shared by `Check` and `Publish`.
fn validate_and_sign(
    ctx: &Ctx<'_>,
    id: &str,
    mut content: Value,
    live_check: bool,
) -> Result<CheckedDocument, Vec<String>> {
    let repo_path = infer_repo_path(id);

    // 1. Standalone tables (`market/goods.json`, `resources.json`, `tools/*.json`)
    if content.get("manifest").is_none() {
        let encoded = serde_json::to_string_pretty(&content).map_err(|e| vec![e.to_string()])?;
        if id == "market/goods.json" || repo_path == "market/goods.json" {
            GoodsTable::load("goods.json", &encoded).map_err(|e| vec![e.to_string()])?;
            let digest = goods_digest(&encoded)
                .map_err(|e| vec![e.to_string()])?
                .to_hex();
            return Ok(CheckedDocument {
                pack: None,
                signed_doc: content,
                digest: Some(digest),
                cascade: Vec::new(),
            });
        }
        if id == "resources.json" || repo_path == "resources.json" {
            ResourceTable::load("resources.json", &encoded).map_err(|e| vec![e.to_string()])?;
            return Ok(CheckedDocument {
                pack: None,
                signed_doc: content,
                digest: None,
                cascade: Vec::new(),
            });
        }
        if id.starts_with("tools/") || repo_path.starts_with("tools/") {
            ToolDef::load(&repo_path, &encoded).map_err(|e| vec![e.to_string()])?;
            let digest = tool_digest(&encoded)
                .map_err(|e| vec![e.to_string()])?
                .to_hex();
            return Ok(CheckedDocument {
                pack: None,
                signed_doc: content,
                digest: Some(digest),
                cascade: Vec::new(),
            });
        }
        return Err(vec![format!(
            "{id}: document has no `manifest` and is not a recognized standalone table"
        )]);
    }

    // 2. Signed Pack (`mush_content::Pack` / `crate::pack::RulesPack`)
    reconcile_requires(ctx, &mut content);

    // Set a placeholder 64-hex digest so `mush_content::Pack` can parse and compute its canonical digest
    if let Some(manifest) = content.get_mut("manifest").and_then(Value::as_object_mut) {
        manifest.insert("digest".to_string(), Value::String("0".repeat(64)));
    }
    let unsigned_pack: mush_content::Pack =
        serde_json::from_value(content.clone()).map_err(|e| vec![e.to_string()])?;
    let digest = unsigned_pack
        .computed_digest()
        .map_err(|e| vec![e.to_string()])?;
    let digest_hex = digest.to_hex();
    if let Some(manifest) = content.get_mut("manifest").and_then(Value::as_object_mut) {
        manifest.insert("digest".to_string(), Value::String(digest_hex.clone()));
    }
    let signed_bytes = serde_json::to_string_pretty(&content).map_err(|e| vec![e.to_string()])?;
    let pack = mush_content::load_pack(&signed_bytes).map_err(|e| vec![e.to_string()])?;

    let mut refusals = Vec::new();

    // Entry-level offline checks
    for (entry_id, entry) in pack.entries() {
        match entry.kind().as_str() {
            "npc" | "lore" => {
                if let Err(err) = mush_npc::content::validate_entry(entry_id, entry) {
                    refusals.push(err);
                }
            }
            _ => {}
        }
    }

    // If this is a `rules_definition` pack or contains owned tacsim entry kinds, validate via `crate::pack::load_pack`
    let has_owned_kind = pack
        .entries()
        .values()
        .any(|e| crate::pack::OWNED_KINDS.contains(&e.kind().as_str()));
    if pack.manifest().kind() == PackKind::RulesDefinition || has_owned_kind {
        match crate::pack::load_pack(&signed_bytes) {
            Ok(rules_pack) => {
                if pack.manifest().kind() == PackKind::RulesDefinition
                    && pack.manifest().id().as_str() != crate::hardware::PACK
                {
                    // Check catalog build for definition packs unless replacing an existing shipped pack version
                    let mut cfg = TacsimConfig::default();
                    let is_shipped_id = fielded_sources().iter().any(|src| {
                        crate::pack::load_pack(&src.bytes)
                            .is_ok_and(|p| p.manifest().id() == rules_pack.manifest().id())
                    });
                    if !is_shipped_id {
                        cfg.packs.push(crate::PackSource {
                            name: repo_path.clone(),
                            bytes: signed_bytes.clone(),
                        });
                        if let Err(err) = verify_sources_with(&cfg, &[]).and_then(|loaded| {
                            Catalog::build_fielded(&loaded, crate::NS).map_err(|e| e.to_string())
                        }) {
                            refusals.push(err);
                        }
                    } else {
                        // Replace matching pack id in fielded_sources and verify + build catalog
                        let mut loaded = Vec::new();
                        for src in fielded_sources() {
                            if let Ok(p) = crate::pack::load_pack(&src.bytes) {
                                if p.manifest().id() == rules_pack.manifest().id() {
                                    loaded.push(LoadedPack {
                                        source: repo_path.clone(),
                                        pack: rules_pack.clone(),
                                    });
                                } else {
                                    loaded.push(LoadedPack {
                                        source: src.name,
                                        pack: p,
                                    });
                                }
                            }
                        }
                        if let Err(err) = Catalog::build(&loaded, crate::NS) {
                            refusals.push(err.to_string());
                        }
                    }
                }
            }
            Err(err) => refusals.push(err.to_string()),
        }
    }

    // Live world applier checks when requested and world has ContentState
    if live_check
        && matches!(
            pack.manifest().kind(),
            PackKind::CampaignSeed | PackKind::Neighborhood | PackKind::Region | PackKind::Site
        )
        && ctx.w.module_state::<mush_content::ContentState>().is_some()
        && let Err(err) = mush_content::check_pack(ctx, &pack)
    {
        refusals.push(err.to_string());
    }

    if !refusals.is_empty() {
        return Err(refusals);
    }

    let cascade = compute_cascade(&pack);
    Ok(CheckedDocument {
        pack: Some(pack),
        signed_doc: content,
        digest: Some(digest_hex),
        cascade,
    })
}

/// Automatically ensure `manifest.requires` includes any `room_pack` or `npc_pack`
/// referenced by `entries` (using the live latch version or shipped version).
fn reconcile_requires(ctx: &Ctx<'_>, content: &mut Value) {
    let mut needed: BTreeSet<String> = BTreeSet::new();
    let self_id = content
        .get("manifest")
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if let Some(entries) = content.get("entries").and_then(Value::as_object) {
        for entry in entries.values() {
            let Some(def) = entry.get("def") else {
                continue;
            };
            for field in ["room_pack", "npc_pack"] {
                if let Some(dep) = def.get(field).and_then(Value::as_str)
                    && !dep.is_empty()
                    && dep != self_id
                {
                    needed.insert(dep.to_string());
                }
            }
            if let Some(prereq) = def.get("prerequisite").and_then(Value::as_object)
                && let Some(q) = prereq.get("quest").and_then(Value::as_str)
                && let Some((dep_pack, _)) = q.split_once('/')
                && dep_pack != self_id
            {
                needed.insert(dep_pack.to_string());
            }
            if let Some(cps) = def.get("checkpoints").and_then(Value::as_array) {
                for cp in cps {
                    if let Some(rp) = cp.get("room_pack").and_then(Value::as_str)
                        && !rp.is_empty()
                        && rp != self_id
                    {
                        needed.insert(rp.to_string());
                    }
                    if let Some(obj) = cp.get("objective").and_then(Value::as_object)
                        && let Some(np) = obj.get("npc_pack").and_then(Value::as_str)
                        && !np.is_empty()
                        && np != self_id
                    {
                        needed.insert(np.to_string());
                    }
                }
            }
        }
    }

    if needed.is_empty() {
        return;
    }

    let content_state = ctx.w.module_state::<mush_content::ContentState>();
    let Some(manifest) = content.get_mut("manifest").and_then(Value::as_object_mut) else {
        return;
    };
    let req_arr = manifest
        .entry("requires")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(reqs) = req_arr.as_array_mut() else {
        return;
    };

    for dep_id in needed {
        let already = reqs
            .iter()
            .any(|r| r.get("id").and_then(Value::as_str) == Some(dep_id.as_str()));
        if !already {
            let version = resolve_pack_version(content_state, &dep_id);
            reqs.push(json!({
                "id": dep_id,
                "version": version,
            }));
        }
    }
}

fn resolve_pack_version(content_state: Option<&mush_content::ContentState>, pack_id: &str) -> u32 {
    if let Some(cs) = content_state
        && let Ok(cid) = CanonicalId::new(pack_id)
        && let Some(latch) = cs.latches.applied(&cid)
        && !latch.is_retired()
    {
        return latch.version();
    }
    for spec in shipped_docs() {
        if spec.id == pack_id
            && let Ok(val) = serde_json::from_str::<Value>(spec.bytes)
            && let Some(v) = val
                .get("manifest")
                .and_then(|m| m.get("version"))
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
        {
            return v;
        }
    }
    1
}

/// Compute any dependent shipped packs whose `requires` pins `pack.manifest().id()`
/// at an older version when `pack`'s version is bumped (e.g. `lane` -> `luna_ceres`).
fn compute_cascade(pack: &mush_content::Pack) -> Vec<Value> {
    let mut cascade = Vec::new();
    let target_id = pack.manifest().id();
    let target_version = pack.manifest().version();

    for spec in shipped_docs() {
        if spec.id == target_id.as_str() {
            continue;
        }
        let Ok(mut dep_val) = serde_json::from_str::<Value>(spec.bytes) else {
            continue;
        };
        let Some(reqs) = dep_val
            .get_mut("manifest")
            .and_then(|m| m.get_mut("requires"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        let mut pinned_old = false;
        for req in reqs.iter_mut() {
            if req.get("id").and_then(Value::as_str) == Some(target_id.as_str())
                && req
                    .get("version")
                    .and_then(Value::as_u64)
                    .is_some_and(|v| v != u64::from(target_version))
            {
                if let Some(obj) = req.as_object_mut() {
                    obj.insert("version".to_string(), json!(target_version));
                }
                pinned_old = true;
            }
        }
        if pinned_old {
            // Bump dependent pack version by 1 and re-sign
            if let Some(manifest) = dep_val.get_mut("manifest").and_then(Value::as_object_mut) {
                let next_ver = manifest
                    .get("version")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .saturating_add(1);
                manifest.insert("version".to_string(), json!(next_ver));
                manifest.insert("digest".to_string(), Value::String("0".repeat(64)));
            }
            if let Ok(unsigned) = serde_json::from_value::<mush_content::Pack>(dep_val.clone())
                && let Ok(digest) = unsigned.computed_digest()
            {
                let hex = digest.to_hex();
                if let Some(manifest) = dep_val.get_mut("manifest").and_then(Value::as_object_mut) {
                    manifest.insert("digest".to_string(), Value::String(hex.clone()));
                }
                cascade.push(json!({
                    "id": spec.id,
                    "repo_path": spec.repo_path,
                    "version": unsigned.manifest().version(),
                    "digest": hex,
                    "pack": dep_val,
                }));
            }
        }
    }
    cascade
}

fn check_document(ctx: &Ctx<'_>, id: &str, content: Value, live_check: bool) -> InvokeResult {
    let repo_path = infer_repo_path(id);
    match validate_and_sign(ctx, id, content, live_check) {
        Ok(CheckedDocument {
            signed_doc,
            digest,
            cascade,
            ..
        }) => InvokeResult::ContentStudio(ContentStudioResponse::Checked {
            id: id.to_string(),
            repo_path,
            refusals: Vec::new(),
            pack: Some(signed_doc),
            digest,
            cascade,
        }),
        Err(refusals) => InvokeResult::ContentStudio(ContentStudioResponse::Checked {
            id: id.to_string(),
            repo_path,
            refusals,
            pack: None,
            digest: None,
            cascade: Vec::new(),
        }),
    }
}

fn publish_document(
    ctx: &mut Ctx<'_>,
    id: &str,
    content: Value,
    relocate_npcs: &[String],
) -> InvokeResult {
    let CheckedDocument {
        pack: maybe_pack,
        signed_doc,
        ..
    } = match validate_and_sign(ctx, id, content, true) {
        Ok(res) => res,
        Err(refusals) => {
            return InvokeResult::ContentStudio(ContentStudioResponse::Published {
                id: id.to_string(),
                version: 0,
                refusals,
                summary: "Validation failed; nothing was published.".to_string(),
                pack: None,
            });
        }
    };

    let Some(pack) = maybe_pack else {
        let ver = signed_doc
            .get("version")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1);
        return InvokeResult::ContentStudio(ContentStudioResponse::Published {
            id: id.to_string(),
            version: ver,
            refusals: Vec::new(),
            summary: format!(
                "Validated standalone table `{}`; download and commit to take effect at next boot.",
                infer_repo_path(id)
            ),
            pack: Some(signed_doc),
        });
    };

    let version = pack.manifest().version();
    match pack.manifest().kind() {
        PackKind::CampaignSeed | PackKind::Neighborhood | PackKind::Region | PackKind::Site => {
            let _ = mush_atlas::stage_pack(ctx, pack.clone());
            // If this is the lane pack, also install it into TacsimStore so `@tacsim/seed` and route reads see it
            if let Ok(rules_pack) =
                crate::pack::load_pack(&serde_json::to_string(&signed_doc).unwrap_or_default())
                && let Some(store) = ctx.state_mut::<TacsimStore>()
            {
                store.loaded.insert(
                    (
                        rules_pack.manifest().id().clone(),
                        rules_pack.manifest().version(),
                    ),
                    LoadedPack {
                        source: format!("<studio {}>", infer_repo_path(id)),
                        pack: rules_pack,
                    },
                );
            }
            match mush_content::apply_pack(ctx, &pack) {
                Ok((applied, report)) => {
                    if let Some(anomaly) = report.anomaly {
                        return InvokeResult::ContentStudio(ContentStudioResponse::Published {
                            id: pack.manifest().id().to_string(),
                            version,
                            refusals: vec![anomaly.clone()],
                            summary: format!("Apply stopped with anomaly: {anomaly}"),
                            pack: Some(signed_doc),
                        });
                    }
                    let relocated = relocate_live_npcs(ctx, &pack, &applied, relocate_npcs);
                    let summary = format!(
                        "Applied {}@{}: {} created, {} updated, {} retired, {} unchanged{}.",
                        pack.manifest().id(),
                        applied.version(),
                        report.created,
                        report.updated,
                        report.retired,
                        report.unchanged,
                        if relocated > 0 {
                            format!(", {relocated} relocated")
                        } else {
                            String::new()
                        }
                    );
                    InvokeResult::ContentStudio(ContentStudioResponse::Published {
                        id: pack.manifest().id().to_string(),
                        version: applied.version(),
                        refusals: Vec::new(),
                        summary,
                        pack: Some(signed_doc),
                    })
                }
                Err(err) => InvokeResult::ContentStudio(ContentStudioResponse::Published {
                    id: pack.manifest().id().to_string(),
                    version,
                    refusals: vec![err.to_string()],
                    summary: format!("Live apply refused: {err}"),
                    pack: None,
                }),
            }
        }
        PackKind::RulesDefinition => {
            if let Ok(rules_pack) =
                crate::pack::load_pack(&serde_json::to_string(&signed_doc).unwrap_or_default())
                && let Some(store) = ctx.state_mut::<TacsimStore>()
            {
                store.loaded.insert(
                    (
                        rules_pack.manifest().id().clone(),
                        rules_pack.manifest().version(),
                    ),
                    LoadedPack {
                        source: format!("<studio {}>", infer_repo_path(id)),
                        pack: rules_pack,
                    },
                );
            }
            InvokeResult::ContentStudio(ContentStudioResponse::Published {
                id: pack.manifest().id().to_string(),
                version,
                refusals: Vec::new(),
                summary: format!(
                    "Registered {}@{}; rules definitions are fieldable after restart.",
                    pack.manifest().id(),
                    version
                ),
                pack: Some(signed_doc),
            })
        }
    }
}

/// Move requested live NPC objects (`npc.<slug>`) to their newly authored `room`
/// on the running world (D-34-9).
fn relocate_live_npcs(
    ctx: &mut Ctx<'_>,
    pack: &mush_content::Pack,
    applied: &mush_content::Applied,
    relocate_npcs: &[String],
) -> usize {
    let mut moved = 0_usize;
    for entry_name in relocate_npcs {
        let Ok(entry_id) = CanonicalId::new(entry_name) else {
            continue;
        };
        let Some(npc_obj) = applied
            .entries()
            .get(&entry_id)
            .and_then(mush_content::AppliedEntry::object)
        else {
            continue;
        };
        let Some(raw_entry) = pack.entry(&entry_id) else {
            continue;
        };
        let Ok(def) = raw_entry.decode::<mush_npc::content::NpcDef>() else {
            continue;
        };
        let Ok(room_cid) = CanonicalId::new(def.room.trim()) else {
            continue;
        };
        let target_room = match &def.room_pack {
            None => applied
                .entries()
                .get(&room_cid)
                .and_then(mush_content::AppliedEntry::object),
            Some(rp) => CanonicalId::new(rp.trim()).ok().and_then(|rp_cid| {
                ctx.w
                    .module_state::<mush_content::ContentState>()
                    .and_then(|cs| cs.latches.applied(&rp_cid))
                    .and_then(|l| l.entries().get(&room_cid))
                    .and_then(mush_content::AppliedEntry::object)
            }),
        };
        if let Some(room_obj) = target_room
            && ctx
                .w
                .get(room_obj)
                .is_some_and(|o| o.kind == ObjKind::Room && !o.flags.contains(Flags::GOING))
            && ctx.w.move_to(npc_obj, room_obj).is_ok()
        {
            moved = moved.saturating_add(1);
        }
    }
    moved
}

/// Structured co-authoring assistant (`LlmFeature::ContentAssist`, D-34-8).
/// Deterministic, bounded synthesis that respects the `BUILDER_LLM` gate and
/// pinned context fields when no external provider is attached, and expands in
/// CS.1/CS.2 for NPC and Quest scopes.
fn assist_document(
    ctx: &Ctx<'_>,
    family: &str,
    scope: &str,
    brief: &str,
    context: &Value,
) -> InvokeResult {
    if !perm::has_power(ctx.w, ctx.actor, Powers::BUILDER_LLM) {
        return error(
            "missing_power",
            "You need the BUILDER_LLM power to use Content Assist.",
        );
    }
    let feature = mush_llm::LlmFeature::ContentAssist;
    let prompt_hash = mush_core::DefinitionDigest::for_canonical_value_with_limit(
        feature.route_key(),
        &json!({
            "family": family,
            "scope": scope,
            "brief": brief,
            "context": context,
        }),
        256 * 1024,
    )
    .map(|d| d.to_hex())
    .unwrap_or_default();

    let patch = synthesize_assist_patch(family, scope, brief, context);
    InvokeResult::ContentStudio(ContentStudioResponse::Assisted {
        patch,
        model: format!("{}:studio-v1", feature.route_key()),
        prompt_hash,
    })
}

fn synthesize_assist_patch(family: &str, scope: &str, brief: &str, context: &Value) -> Value {
    let lower = brief.to_ascii_lowercase();
    match family {
        "npc" => synthesize_npc_patch(scope, brief, &lower, context),
        "quest" => synthesize_quest_patch(scope, brief, &lower, context),
        _ => json!({ "note": brief.trim() }),
    }
}

fn synthesize_npc_patch(scope: &str, brief: &str, lower: &str, context: &Value) -> Value {
    let (port_id, room_pack, default_room, station_label) =
        if lower.contains("stickney") || lower.contains("phobos") {
            (
                "stickney_port",
                "neighborhood.stickney_yards",
                "room.visiting_berth_gallery",
                "Stickney Yards",
            )
        } else if lower.contains("aitken") || lower.contains("luna") {
            (
                "luna_port",
                "neighborhood.aitken_highport",
                "room.arrivals_concourse",
                "Aitken Highport",
            )
        } else if lower.contains("voltaire") || lower.contains("deimos") {
            (
                "deimos_port",
                "neighborhood.voltaire_anchorage",
                "room.docking_spine",
                "Voltaire Anchorage",
            )
        } else if lower.contains("ligeia") || lower.contains("titan") {
            (
                "titan_port",
                "neighborhood.ligeia_terminal",
                "room.transfer_concourse",
                "Ligeia Terminal",
            )
        } else {
            (
                "ceres_port",
                "neighborhood.ceres_freeport",
                "room.arrivals",
                "Ceres Freeport",
            )
        };

    let mut duties = Vec::new();
    if lower.contains("repair")
        || lower.contains("maint")
        || lower.contains("salvage")
        || lower.contains("recover")
    {
        duties.push("recovery");
    }
    if lower.contains("min") || lower.contains("drill") || lower.contains("excavat") {
        duties.push("mining");
    }
    if lower.contains("combat") || lower.contains("gun") || lower.contains("escort") {
        duties.push("combat");
    }
    if duties.is_empty() {
        duties.push("recovery");
    }

    let persona = format!(
        "Seasoned {station_label} specialist ({}). Methodical, direct, and attentive to hull wear, seal tolerances, and practical berth routines.",
        brief.trim()
    );
    let background = format!(
        "Worked the docks and repair gantries around {station_label} for years before taking independent contracts. Known along the corridor for keeping ageing drives and frames running under tight turnaround schedules."
    );
    let speech_style =
        "Clipped, practical dock-register; cites pressures, seals, and shift hours without wasted words."
            .to_string();
    let goals = vec![
        format!("Sign onto a well-maintained hull operating out of {station_label}."),
        "Keep every frame, seal, and recovery rig within tolerance before undocking.".to_string(),
    ];
    let knows = vec![
        format!("{station_label} berth procedures, maintenance schedules, and shift rosters."),
        "Practical field recovery, hull patching, and telemetry diagnostics.".to_string(),
    ];
    let does_not_know = vec![
        "Classified House Registry seal-mark internals.".to_string(),
        "Off-corridor fleet movements beyond public berth postings.".to_string(),
    ];
    let fallback = vec![
        "Check the pressure seals first; the rest of the job waits on the gauge.".to_string(),
        format!("If you have a berth at {station_label}, bring the maintenance log to the desk."),
    ];

    match scope {
        "persona" => json!({ "persona": persona }),
        "background" => json!({ "background": background }),
        "speech_style" => json!({ "speech_style": speech_style }),
        "goals" => json!({ "goals": goals }),
        "knows" => json!({ "knows": knows }),
        "does_not_know" => json!({ "does_not_know": does_not_know }),
        "fallback" => json!({ "fallback": fallback }),
        "relocate" => {
            let from_label = context
                .get("from_station")
                .and_then(Value::as_str)
                .unwrap_or("Ceres Freeport");
            let to_label = context
                .get("to_station")
                .and_then(Value::as_str)
                .unwrap_or(station_label);
            let adapt = |text: &str| text.replace(from_label, to_label);
            let old_persona = context
                .get("persona")
                .and_then(Value::as_str)
                .unwrap_or(&persona);
            let old_bg = context
                .get("background")
                .and_then(Value::as_str)
                .unwrap_or(&background);
            json!({
                "persona": adapt(old_persona),
                "background": adapt(old_bg),
            })
        }
        _ => json!({
            "slug": "hana_vos",
            "name": "Hana Vos",
            "room_pack": room_pack,
            "room": default_room,
            "tier": "supporting",
            "profile": {
                "persona": persona,
                "background": background,
                "speech_style": speech_style,
                "goals": goals,
                "knows": knows,
                "does_not_know": does_not_know,
                "fallback": fallback,
                "watch": ["maintenance", "hull", "berth"],
                "interfaces": ["hire"],
                "command_refs": ["+crew"],
            },
            "terms": {
                "schema": 1,
                "port": port_id,
                "mission_scope": "subordinate_operator",
                "duties": duties,
            }
        }),
    }
}

fn synthesize_quest_patch(scope: &str, brief: &str, _lower: &str, _context: &Value) -> Value {
    let intro = format!(
        "\"We need a reliable captain to handle this run ({}) and bring back a clean reading.\"",
        brief.trim()
    );
    let completion =
        "\"That matches the ledger. Your account is credited and the berth desk has your receipt.\""
            .to_string();
    let instructions =
        "Inspect the posted board in the room (`look <detail>`) to record the checkpoint."
            .to_string();
    match scope {
        "introduction" => json!({ "introduction": intro }),
        "completion" => json!({ "completion": completion }),
        "checkpoint_instructions" => json!({ "instructions": instructions }),
        _ => json!({
            "title": "Station Verification Run",
            "destination": "Ceres Freeport",
            "introduction": intro,
            "completion": completion,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_documents_parse_and_roundtrip_through_studio_check()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut w, reg) = mush_testkit::boot_world();
        let staff = w.create(ObjKind::Player, "Staff", GOD, None)?;
        if let Some(obj) = w.get_mut(staff) {
            obj.flags.insert(Flags::WIZARD);
            obj.powers.insert(Powers::BUILDER_LLM);
        }
        let mut out = mush_core::Outbox::new();
        let ctx = Ctx {
            w: &mut w,
            actor: staff,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &reg,
            module: Some(crate::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };

        // Catalog builds cleanly and stays within the 272 KiB wire cap
        let cat_res = build_catalog(&ctx).bounded();
        let InvokeResult::ContentStudio(ContentStudioResponse::Catalog { catalog }) = cat_res
        else {
            return Err("expected Catalog response".into());
        };
        check!(
            catalog
                .get("ports")
                .and_then(Value::as_array)
                .is_some_and(|a| a.len() == 11)
        );
        check!(
            catalog
                .get("npcs")
                .and_then(Value::as_array)
                .is_some_and(|a| a.len() == 56)
        );
        check!(
            catalog
                .get("goods")
                .and_then(Value::as_array)
                .is_some_and(|a| a.len() == 12)
        );

        // Read + Check a shipped campaign_seed pack (`tacsim.crew.ceres`) and a standalone table (`market/goods.json`)
        for doc_id in [
<<<<<<< HEAD
            "tacsim.cast.ceres_freeport",
=======
>>>>>>> feat/cs-0a-content-studio-seam
            "tacsim.crew.ceres",
            "market/goods.json",
            "tacsim.pack.cannon_105",
        ] {
            let read_res = read_document(&ctx, doc_id);
            let InvokeResult::ContentStudio(ContentStudioResponse::Document { content, .. }) =
                read_res
            else {
                return Err(format!("expected Document for {doc_id}").into());
            };
            let check_res = check_document(&ctx, doc_id, content, false);
            let InvokeResult::ContentStudio(ContentStudioResponse::Checked {
                refusals, pack, ..
            }) = check_res
            else {
                return Err(format!("expected Checked for {doc_id}").into());
            };
            check!(refusals.is_empty(), "{doc_id} refusals: {refusals:?}");
            check!(pack.is_some(), "{doc_id} signed pack missing");
        }

        // Assist synthesizes a valid NPC draft when actor holds BUILDER_LLM
        let assist_res = assist_document(
            &ctx,
            "npc",
            "full",
            "maintenance/repair specialist based on ceres, heavily experienced",
            &json!({}),
        );
        let InvokeResult::ContentStudio(ContentStudioResponse::Assisted { patch, .. }) = assist_res
        else {
            return Err("expected Assisted response".into());
        };
        check_eq!(
            patch
                .get("terms")
                .and_then(|t| t.get("port"))
                .and_then(Value::as_str),
            Some("ceres_port")
        );
        Ok(())
    }
}
