//! **Additive structured client sync & port trade feeds** (`port.trade`,
//! `tacsim.content`).
//!
//! Two pull-only panel feeds for graphical 3D clients (such as the Unity URP
//! client):
//!
//! - [`PORT_TRADE_FEED`] (`"port.trade"`): structured port venue, actor liquid
//!   credit balance, visible market asks/bids (via [`mush_market::visible`]),
//!   and the actor's own charter/title hull stowage gauges.
//! - [`CLIENT_CONTENT_FEED`] (`"tacsim.content"`): SHA-256 content manifest and
//!   per-pack JSON retrieval over WebSocket `PanelSubscribe` queries, mirrored
//!   by [`http_content_packs`] for HTTP `GET /api/content/manifest` and
//!   `GET /api/content/packs/{id}`.
//!
//! Neither feed ends in a suffix auto-subscribed by the web SPA or SSH TUI
//! (`.vitals`, `.view`, `.unit`, `.sheet`, `.roll`, `.chart`, `.map`,
//! `.market`), so existing clients ignore both unless they explicitly query
//! them.

use std::sync::{Arc, OnceLock};

use mush_core::module::{ModError, Registrar};
use mush_core::{HookReadContext, ObjId, QueryHandler, World};
use mush_economy::{Amount, EconomyStore, InstrumentId};
use mush_market::{MARKET_PAGE, MarketFilter, MarketStore};
use mush_org::PrincipalId;
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::economy::CREDIT;
use crate::port::board::money;
use crate::port::coop::{EnrolmentStatus, helm_of, name_of, port_name};
use crate::port::market::{PortVenueGate, port_of_venue};
use crate::port::ship::{Berth, berth_of, hull_of_room, hull_stowage_json, hull_view};
use crate::store::TacsimStore;

/// Pulled panel feed returning structured port market listings, actor credits,
/// and own-hull stowage gauges without writing to scrollback.
pub const PORT_TRADE_FEED: &str = "port.trade";

/// Pulled panel feed returning the dynamic client content pack manifest and
/// individual pack JSON payloads by id.
pub const CLIENT_CONTENT_FEED: &str = "tacsim.content";

/// Schema version for both client feeds.
pub const SCHEMA_VERSION: u32 = 1;

/// Raw client-relevant JSON packs compiled into the server binary (`(id, category, json)`).
const CLIENT_PACK_SOURCES: &[(&str, &str, &str)] = &[
    // OOC & starter neighborhoods (`mush-atlas`)
    (
        "neighborhood.ooc_area",
        "neighborhood",
        mush_atlas::starter::OOC_AREA_JSON,
    ),
    (
        "neighborhood.staff_lobby",
        "neighborhood",
        mush_atlas::starter::STAFF_LOBBY_JSON,
    ),
    // Station & surface neighborhoods (`tacsim`)
    (
        "neighborhood.ceres_freeport",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.ceres_freeport.json"),
    ),
    (
        "neighborhood.stickney_yards",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.stickney_yards.json"),
    ),
    (
        "neighborhood.aitken_highport",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.aitken_highport.json"),
    ),
    (
        "neighborhood.voltaire_anchorage",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.voltaire_anchorage.json"),
    ),
    (
        "neighborhood.ligeia_terminal",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.ligeia_terminal.json"),
    ),
    (
        "neighborhood.kirkwood_depot",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.kirkwood_depot.json"),
    ),
    (
        "neighborhood.ahuna_flank",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.ahuna_flank.json"),
    ),
    (
        "neighborhood.occator_rim",
        "neighborhood",
        include_str!("../content/atlas/neighborhood.occator_rim.json"),
    ),
    // Economy & trade lane
    (
        "goods",
        "market",
        include_str!("../content/market/goods.json"),
    ),
    (
        "ceres_phobos",
        "lane",
        include_str!("../content/lane/ceres_phobos.json"),
    ),
    // Ship, rover & combat unit definitions
    ("dray", "unit", include_str!("../content/units/dray.json")),
    (
        "dray_p",
        "unit",
        include_str!("../content/units/dray_p.json"),
    ),
    (
        "corvette",
        "unit",
        include_str!("../content/units/corvette.json"),
    ),
    (
        "halcyon",
        "unit",
        include_str!("../content/units/halcyon.json"),
    ),
    (
        "tv_prospector",
        "unit",
        include_str!("../content/units/tv_prospector.json"),
    ),
    (
        "tv_escort",
        "unit",
        include_str!("../content/units/tv_escort.json"),
    ),
    (
        "tv_tender",
        "unit",
        include_str!("../content/units/tv_tender.json"),
    ),
    (
        "line_tank",
        "unit",
        include_str!("../content/units/line_tank.json"),
    ),
    (
        "line_tank_laser",
        "unit",
        include_str!("../content/units/line_tank_laser.json"),
    ),
    (
        "line_tank_thermal",
        "unit",
        include_str!("../content/units/line_tank_thermal.json"),
    ),
    (
        "line_tank_ecm",
        "unit",
        include_str!("../content/units/line_tank_ecm.json"),
    ),
    (
        "line_tank_eccm",
        "unit",
        include_str!("../content/units/line_tank_eccm.json"),
    ),
    (
        "heavy_mk1",
        "unit",
        include_str!("../content/units/heavy_mk1.json"),
    ),
    (
        "biped_mk1",
        "unit",
        include_str!("../content/units/biped_mk1.json"),
    ),
    (
        "biped_mk2",
        "unit",
        include_str!("../content/units/biped_mk2.json"),
    ),
    (
        "scout_mk1",
        "unit",
        include_str!("../content/units/scout_mk1.json"),
    ),
    (
        "drone_mk1",
        "unit",
        include_str!("../content/units/drone_mk1.json"),
    ),
    // Weapons
    (
        "cannon_105",
        "weapon",
        include_str!("../content/weapons/cannon_105.json"),
    ),
    (
        "laser_mk1",
        "weapon",
        include_str!("../content/weapons/laser_mk1.json"),
    ),
    (
        "autocannon_30",
        "weapon",
        include_str!("../content/weapons/autocannon_30.json"),
    ),
    (
        "missile_lr",
        "weapon",
        include_str!("../content/weapons/missile_lr.json"),
    ),
    (
        "mg_coax",
        "weapon",
        include_str!("../content/weapons/mg_coax.json"),
    ),
    // Simulator arena maps (atlas terrain)
    (
        "arena_atlas.proving_ground",
        "arena_atlas",
        include_str!("../content/atlas/proving_ground.json"),
    ),
    (
        "arena_atlas.mars_canyon",
        "arena_atlas",
        include_str!("../content/atlas/mars_canyon.json"),
    ),
    (
        "arena_atlas.winter_valley",
        "arena_atlas",
        include_str!("../content/atlas/winter_valley.json"),
    ),
];

/// Precomputed SHA-256 entry for one client content pack.
#[derive(Clone, Debug)]
pub struct ClientPackEntry {
    /// Stable pack identifier (`"neighborhood.ceres_freeport"`, `"dray"`, `"goods"`, ...).
    pub id: &'static str,
    /// Pack category (`"neighborhood"`, `"unit"`, `"weapon"`, `"market"`, `"lane"`, `"arena_atlas"`).
    pub category: &'static str,
    /// Lowercase hex SHA-256 of `json`.
    pub sha256: Arc<str>,
    /// Raw UTF-8 JSON payload.
    pub json: Arc<str>,
}

/// Cached bundle of all client content packs and their serialized manifest JSON.
#[derive(Debug)]
pub struct ClientContentBundle {
    /// Serialized HTTP manifest JSON (`{ "version": 1, "packs": [...] }`).
    pub manifest_json: Arc<str>,
    /// Manifest entries as a `serde_json::Value` array for WebSocket frames.
    pub manifest_packs_value: Value,
    /// Individual pack entries.
    pub entries: Vec<ClientPackEntry>,
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

/// Lazily computed static bundle of client content packs and SHA-256 digests.
#[must_use]
pub fn client_content_bundle() -> &'static ClientContentBundle {
    static BUNDLE: OnceLock<ClientContentBundle> = OnceLock::new();
    BUNDLE.get_or_init(|| {
        let mut entries = Vec::with_capacity(CLIENT_PACK_SOURCES.len());
        let mut pack_rows = Vec::with_capacity(CLIENT_PACK_SOURCES.len());
        for &(id, category, raw_json) in CLIENT_PACK_SOURCES {
            let sha = sha256_hex(raw_json.as_bytes());
            pack_rows.push(json!({
                "id": id,
                "category": category,
                "sha256": sha,
                "bytes": raw_json.len(),
            }));
            entries.push(ClientPackEntry {
                id,
                category,
                sha256: Arc::from(sha),
                json: Arc::from(raw_json),
            });
        }
        let manifest_packs_value = Value::Array(pack_rows);
        let manifest_doc = json!({
            "version": SCHEMA_VERSION,
            "packs": manifest_packs_value,
        });
        let manifest_str = serde_json::to_string(&manifest_doc)
            .unwrap_or_else(|_| "{\"version\":1,\"packs\":[]}".to_owned());
        ClientContentBundle {
            manifest_json: Arc::from(manifest_str),
            manifest_packs_value,
            entries,
        }
    })
}

/// One HTTP-served content pack entry: `(id, sha256, json)`.
pub type HttpContentPackEntry = (String, Arc<str>, Arc<str>);

/// Exported tuple for `WebBridge::with_content_packs`: `(manifest_json, packs)`.
#[must_use]
pub fn http_content_packs() -> (Arc<str>, Vec<HttpContentPackEntry>) {
    let bundle = client_content_bundle();
    let packs = bundle
        .entries
        .iter()
        .map(|e| (e.id.to_owned(), Arc::clone(&e.sha256), Arc::clone(&e.json)))
        .collect();
    (Arc::clone(&bundle.manifest_json), packs)
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
    reg.query_handler(CLIENT_CONTENT_FEED, Arc::new(ClientContentQuery))?;
    Ok(())
}

struct ClientContentQuery;

impl QueryHandler for ClientContentQuery {
    fn query(&self, _w: &World, _actor: ObjId, params: &Value) -> Result<OutputEvent, RichText> {
        let bundle = client_content_bundle();
        let op = params
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("manifest");
        let data = if op == "pack" {
            let req_id = params.get("id").and_then(Value::as_str).unwrap_or("");
            match bundle.entries.iter().find(|e| e.id == req_id) {
                Some(entry) => json!({
                    "schema": { "id": CLIENT_CONTENT_FEED, "version": SCHEMA_VERSION },
                    "op": "pack",
                    "found": true,
                    "id": entry.id,
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
                "packs": bundle.manifest_packs_value,
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
    let who = PrincipalId::Object(actor);
    let available_minor = economy
        .snapshot()
        .accounts_of(&who)
        .find(|a| a.instrument == instrument)
        .map_or(0_i64, |a| {
            a.posted_balance
                .minor()
                .saturating_sub(a.held_amount.minor())
                .max(0)
        });
    let text = mush_economy::render_amount(
        economy.snapshot(),
        &instrument,
        Amount::from_minor(available_minor),
    );
    (available_minor, text)
}

fn resolve_actor_venue(w: &World, actor: ObjId) -> Option<mush_market::MarketId> {
    let loc = w.get(actor).and_then(|o| o.location)?;
    use mush_market::VenueGate as _;
    if let Some(v) = PortVenueGate.venue_of_room(w, loc) {
        return Some(v);
    }
    // If aboard a docked hull, resolve the port venue of the hull's berth.
    if let Some((view, _)) = hull_of_room(w, loc)
        && let Berth::Docked { def, .. } = berth_of(w, view.hull())
    {
        return mush_market::MarketId::new(def.venue.as_str());
    }
    None
}

fn build_port_trade_frame(ctx: Option<&HookReadContext<'_>>, w: &World, actor: ObjId) -> Value {
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
    let venue = resolve_actor_venue(w, actor);
    let venue_port_label = venue
        .as_ref()
        .and_then(|v| port_of_venue(w, v))
        .map(|(def, _)| port_name(def));

    let mut listings_json = Vec::new();
    if let (Some(market_store), Some(v)) = (w.module_state::<MarketStore>(), venue.as_ref()) {
        let filter = MarketFilter::Market(v.clone());
        let page = mush_market::visible(w, actor, Some(&filter), MARKET_PAGE, None);
        let goods = store.goods();
        for id in &page.ids {
            let Some(l) = market_store.listing(*id) else {
                continue;
            };
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
            let vendor_name = market_store.vendor(l.vendor).map_or_else(
                || format!("vendor {}", l.vendor),
                |vend| vend.name.to_string(),
            );
            listings_json.push(json!({
                "id": l.id.0,
                "side": side,
                "good": good_code,
                "good_label": good_label,
                "vendor_name": vendor_name,
                "unit_price_minor": l.price.minor(),
                "price_text": money(w, l.price),
                "available_qty": l.quantity,
            }));
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

    json!({
        "schema": { "id": PORT_TRADE_FEED, "version": SCHEMA_VERSION },
        "enabled": true,
        "venue": venue.as_ref().map(mush_market::MarketId::as_str),
        "port_name": venue_port_label,
        "balance_minor": balance_minor,
        "balance_text": balance_text,
        "listings": listings_json,
        "ship": ship_json,
    })
}

impl QueryHandler for PortTradeQuery {
    fn query(&self, w: &World, actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(PORT_TRADE_FEED),
            kind: PanelFrameKind::Keyframe,
            data: build_port_trade_frame(None, w, actor),
        })
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        _params: &Value,
    ) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(PORT_TRADE_FEED),
            kind: PanelFrameKind::Keyframe,
            data: build_port_trade_frame(Some(context), context.world(), actor),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn client_content_bundle_hashes_and_parses_all_packs() -> TestResult {
        let bundle = client_content_bundle();
        check!(!bundle.entries.is_empty());
        let parsed_manifest: Value = serde_json::from_str(&bundle.manifest_json)?;
        check_eq!(parsed_manifest["version"], json!(SCHEMA_VERSION));
        for entry in &bundle.entries {
            check_eq!(entry.sha256.len(), 64);
            check!(!entry.json.is_empty());
            let parsed: Value = serde_json::from_str(&entry.json)?;
            check!(parsed.is_object());
        }
        Ok(())
    }

    #[test]
    fn client_content_query_answers_manifest_and_pack() -> TestResult {
        let tw = mush_testkit::TestWorld::new();
        let w = &tw.engine().world;
        let q = ClientContentQuery;
        let Ok(ev) = q.query(w, ObjId(1), &json!({"op": "manifest"})) else {
            return crate::test_checks::failed("manifest query failed");
        };
        let OutputEvent::Panel { feed, data, .. } = ev else {
            return crate::test_checks::failed("expected panel frame");
        };
        check_eq!(feed.as_str(), CLIENT_CONTENT_FEED);
        check_eq!(data["op"], json!("manifest"));
        check!(data["packs"].as_array().is_some_and(|a| !a.is_empty()));

        let Ok(dray) = q.query(w, ObjId(1), &json!({"op": "pack", "id": "dray"})) else {
            return crate::test_checks::failed("pack query failed");
        };
        let OutputEvent::Panel { data: pdata, .. } = dray else {
            return crate::test_checks::failed("expected panel frame");
        };
        check_eq!(pdata["found"], json!(true));
        check_eq!(pdata["id"], json!("dray"));
        check!(pdata["json"].as_str().is_some_and(|s| s.contains("dray")));
        Ok(())
    }
}
