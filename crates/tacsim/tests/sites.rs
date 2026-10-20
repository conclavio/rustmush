//! **Doc 20 cut SM.2 acceptance — sites and landing** (doc 20 §1 beats 2–3, §3.1, §3.3, §3.4,
//! §9.5, §11's Site, Landing and Hop rows; D-20-2, D-20-3, D-20-14; Q-20-7, Q-20-10; the SM.2
//! brief's lead overrides 1–5 and decisions 1–23).
//!
//! Lane v9 puts six mining sites on the corridor's map — five on Ceres a hop from the Freeport,
//! Rheasilvia Commons on Vesta a passage away — each a vendor-less port entry with a `site`
//! block, an atlas region and a one-room neighborhood whose pad carries `port.dock`. A chartered
//! Dray flies to one, settles on its pad, sits there on its own stores (charged at the lift,
//! lead ruling 5) and lifts home. The pure figures of the landing law and the hop are in
//! `tests/landing.rs`; the recall's week of hops in `mush-server/tests/ic_buyout_dormancy.rs`.
//!
//! Composed on the composition root (`commerce.rs`'s `Roster`: the test world's layer with the
//! game content, an operator lane, a shelf and an extra lane as parameters; `voyage.rs`'s
//! helpers). Every boot asserts no namespace write was refused; every drain asserts no kept
//! tacsim submission is GOD's. The world tests fly at world rate 1, so every IC instant a test
//! names is the instant the world reads.

mod support;

#[path = "support/geology_keyed.rs"]
mod geology_keyed;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mush_atlas::{AtlasState, ValidateContext, validate};
use mush_content::{ContentState, parse_pack};
use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, Flags, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{EconomyConfig, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::Value;
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION, SITE_SOURCES};
use tacsim::pack::{PackEntry, RulesPack, SITE_ROLE, load_pack, rules_pack_of};
use tacsim::port::apply::room_by_key;
use tacsim::port::executor::WAITING_LINE;
use tacsim::port::passage::route_landing_floor;
use tacsim::port::ship::{Berth, LEAVE_PAD, berth_of};
use tacsim::port::site::{
    SURVEY_LINE, SURVEY_UNAVAILABLE, TENURE_LINE, geology_spec, site_province,
};
use tacsim::port::voyage::{
    LANDING_REFUSED, LIFT_SEALS, LIFT_USAGE, SETTLED_NOTICE, SITE_CLOSED, SITE_HOME,
    SITE_STORES_SHORT, SITE_TANK_SHORT, THROTTLE_LATER_SITE, THROTTLE_NONE_SITE, Voyage,
    VoyageState,
};
use tacsim::port::{
    BLOCK_LETTERS, Binding, HullDrive, HullRow, LineState, PortDef, PortHandles, PortMap,
    PortMapState, Tenure, Ticket, TicketId, WINDOW_DAYS, room_key,
};
use tacsim::surface::geology::{BodyGeology, GeologyTable, GeologyTableWire, province_at};
use tacsim::surface::ground::{site_geology, site_ground_sync};
use tacsim::surface::reading::read_r0;
use tacsim::surface::{GeologyBoot, OreFamily, Word, installed_bodies};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{CERES_TABLE, KEY_1, VESTA_TABLE, boot_keyed, mac, secret};
use support::ground::{UNSIGNED, sign};
use support::sites::{SITE_PORTS, SITE_VENUES, strip_sites};

// ---------------------------------------------------------------------------
// The content pins
// ---------------------------------------------------------------------------

fn content_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content")
}

fn writing() -> bool {
    std::env::var("TACSIM_WRITE_CONTENT").is_ok_and(|v| v == "1")
}

/// The twelve site packs' files, in [`SITE_SOURCES`] order.
fn site_files() -> Vec<String> {
    SITE_SOURCES
        .iter()
        .map(|(name, _)| format!("atlas/{name}.json"))
        .collect()
}

/// The pinned digests, in [`SITE_SOURCES`] order. Re-sign in place with
/// `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test sites sm2_the_site_packs`.
const PINNED: [(&str, &str); 12] = [
    (
        "region.freeport_commons",
        "0128a8b67e95da3dfef1c587208f249f736e79e37925a033b8ff3fd53ce4a0ff",
    ),
    (
        "neighborhood.freeport_commons",
        "c112498ef5d5228518434ba336bea4809b91d118a1b08c0ec6275f0ad2b63375",
    ),
    (
        "region.kerwan_floor",
        "0a0c58ab4fc1a35b805bcf7d5b6634c148662c306ce0adfaa7b5c3ae281ee015",
    ),
    (
        "neighborhood.kerwan_floor",
        "3501aa8a84a862c4a4c3e06595e3d5f434b5fcd46b6b5e03ab02f00137129c6e",
    ),
    (
        "region.ahuna_flank",
        "0669f6eba46df999d9e92c92f3b53120046a2fcaac251377b8ca32fa8d31b0f3",
    ),
    (
        "neighborhood.ahuna_flank",
        "9540fa38dd76ba938a3a0ec8eff9908cb17a892123fdcc65df07617781d72614",
    ),
    (
        "region.juling_shadow",
        "8c1b558d1e6aa3430284c7730687c8fb7a15a34a5339a7401816fcd1d1a16ec7",
    ),
    (
        "neighborhood.juling_shadow",
        "97655785413243d6e9bac03a59d9af99a2db6bf7d0029db8cb87f969fad05c5a",
    ),
    (
        "region.occator_rim",
        "df4e7b42c6b4b3dd924c6e5fe21cc06be178a23aeb8e0e80c423a2f94d5b8a3d",
    ),
    (
        "neighborhood.occator_rim",
        "18ac2c2dabf0a50e30fa6ced16b5a6ccfaefb3ddf09d382f08860bfce316a517",
    ),
    (
        "region.rheasilvia_commons",
        "2ead006147af1a5daddfdc2d86f6e43e89c911e78e8046ba0dcea3aa77b74466",
    ),
    (
        "neighborhood.rheasilvia_commons",
        "f61a052ca48765dd22fd0d1000e5aee9853e954c5a0ebfa3db2b98fad4ec9487",
    ),
];

/// Lane v8's published digest (doc 19 cut A.8), which `strip_sites` rebuilds.
const LANE_V8_DIGEST: &str = "be733686a1630175e541346c693fcb2cce9f658db876121c73c4d476e04ca330";

/// One site as the brief's table authors it.
struct SiteRow {
    loc: &'static str,
    label: &'static str,
    body: &'static str,
    lat: i32,
    lon: i32,
    venue: &'static str,
    entry: &'static str,
    tenure: Tenure,
}

/// The six sites (the brief's item 3, with lead override 2's Occator Rim on the crater's north
/// rim: 25.4 N, about 46 km north of the centre at 19.82 N, Ceres's 470 km radius giving
/// about 8.2 km a degree — the fixer's correction of the override's 21.3 N, which stood on
/// the floor 12 km from the centre).
const SITES: [SiteRow; 6] = [
    SiteRow {
        loc: "freeport_commons",
        label: "Freeport Commons",
        body: "ceres",
        lat: 20_300,
        lon: 240_400,
        venue: "commons",
        entry: "commons_port",
        tenure: Tenure::Commons,
    },
    SiteRow {
        loc: "kerwan_floor",
        label: "Kerwan Floor",
        body: "ceres",
        lat: -10_800,
        lon: 123_900,
        venue: "kerwan",
        entry: "kerwan_port",
        tenure: Tenure::Open,
    },
    SiteRow {
        loc: "ahuna_flank",
        label: "Ahuna Flank",
        body: "ceres",
        lat: -10_500,
        lon: 316_200,
        venue: "ahuna",
        entry: "ahuna_port",
        tenure: Tenure::Open,
    },
    SiteRow {
        loc: "juling_shadow",
        label: "Juling Shadow",
        body: "ceres",
        lat: -35_900,
        lon: 168_600,
        venue: "juling",
        entry: "juling_port",
        tenure: Tenure::Open,
    },
    SiteRow {
        loc: "occator_rim",
        label: "Occator Rim",
        body: "ceres",
        lat: 25_400,
        lon: 239_300,
        venue: "occator",
        entry: "occator_port",
        tenure: Tenure::Open,
    },
    SiteRow {
        loc: "rheasilvia_commons",
        label: "Rheasilvia Commons",
        body: "vesta",
        lat: -75_000,
        lon: 301_000,
        venue: "rheasilvia",
        entry: "rheasilvia_port",
        tenure: Tenure::Commons,
    },
];

/// The Rheasilvia passage's `base_s`, read off the engine's epoch chord (226 km/s).
const RHEASILVIA_BASE_S: u32 = 3_127_945;

/// The Dray's dry mass, kilograms.
const DRY_KG: u64 = 240_000;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

fn lane() -> RulesPack {
    load_pack(LANE_JSON).expect("the lane verifies")
}

/// The lane at `version` with `edit` applied, re-signed (`commerce.rs`'s `lane_variant`).
fn lane_variant(version: u32, edit: impl FnOnce(&mut Value)) -> String {
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = serde_json::json!(version);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

fn port_def(entry: &str) -> PortDef {
    match lane().entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The rig (commerce.rs's Roster; voyage.rs's helpers)
// ---------------------------------------------------------------------------

/// How a test roster is composed: `commerce.rs`'s (no sheet, the test world's layer, the
/// shipped game content), an operator lane in place of the shipped one, a shelf, or an extra
/// lane version beside the shipped one.
#[derive(Clone, Default)]
struct Roster {
    seed_lane: Option<String>,
    shelf: Option<Vec<mush_content::Pack>>,
    extra_lane: Option<String>,
    /// `[tacsim.time.ephemeris].rate`: 0 (the default) holds the sky constant.
    sky_rate: u32,
}

impl Roster {
    fn bare() -> Roster {
        Roster::default()
    }

    fn seeded(lane: String) -> Roster {
        Roster {
            seed_lane: Some(lane),
            ..Roster::default()
        }
    }

    /// The shipped roster under a moving sky (`voyage.rs`'s `sky_rate`).
    fn moving_sky() -> Roster {
        Roster {
            sky_rate: 1_000,
            ..Roster::default()
        }
    }

    fn modules(&self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let llm = Arc::new(
            mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
                .expect("a disabled LLM service composes"),
        );
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let mut cfg = TacsimConfig {
            enabled: true,
            ..TacsimConfig::default()
        };
        cfg.time.ephemeris.rate = self.sky_rate;
        let game_content = self.seed_lane.is_none();
        if let Some(lane) = &self.extra_lane {
            cfg.seeds.push(PackSource {
                name: "ops/lane_next.json".to_owned(),
                bytes: lane.clone(),
            });
        }
        if let Some(lane) = &self.seed_lane {
            cfg.seeds.push(PackSource {
                name: "ops/lane.json".to_owned(),
                bytes: lane.clone(),
            });
        }
        let (mut mods, inventory) = comp::server_modules_with_tacsim(
            mush_weather::WeatherConfig::default(),
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            comp::recovery_policy(&cfg.recovery),
        );
        if let Some(shelf) = &self.shelf {
            let at = mods
                .iter()
                .position(|m| m.name() == mush_atlas::NS)
                .expect("the atlas composes");
            let mut full =
                tacsim::definitions::content::station_packs().expect("the station packs");
            full.extend(shelf.iter().cloned());
            mods[at] = Box::new(
                mush_atlas::AtlasModule::with_config(mush_atlas::AtlasConfig::default())
                    .with_shelf(full),
            );
        }
        mods.push(comp::worldgen_module_with(
            Arc::clone(&llm),
            mush_worldgen::WorldgenConfig::default(),
        ));
        mods.push(comp::npc_module_with(
            comp::npc_agent_adapter(),
            &mush_config::config::NpcCfg::default(),
        ));
        mods.push(comp::tacsim_test_world_module_wired(
            cfg,
            game_content,
            inventory,
        ));
        match &self.seed_lane {
            None => mods
                .push(comp::content_module_with(true, true).expect("the content module composes")),
            Some(lane) => {
                let mut packs = comp::boot_pack_list(true, true).expect("the boot list");
                packs.retain(|p| p.manifest().id().as_str() != LANE_PACK);
                packs.push(mush_content::load_pack(lane).expect("the seed lane verifies"));
                let mut appliers = mush_content::Appliers::new();
                mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
                mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
                tacsim::register_appliers(&mut appliers).expect("tacsim appliers");
                mods.push(Box::new(
                    mush_content::ContentModule::new()
                        .with_appliers(appliers)
                        .with_boot_packs(packs),
                ));
            }
        }
        mods
    }
}

/// A fixed gateway-account oracle (`match_lifecycle.rs`'s precedent).
#[derive(Default)]
struct MapOracle(Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

/// A booted world, its roster, its oracle and the cumulative store.
struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    roster: Roster,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed: BTreeMap<(SmolStr, SmolStr), serde_json::Value>,
}

fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

fn install_oracle(tw: &mut TestWorld, oracle: &Arc<MapOracle>) {
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(oracle.clone());
}

fn with_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x52; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// Boot `roster` without settling the capital or firing the port tick.
fn boot_unsettled(roster: Roster) -> Rig {
    let mut tw = TestWorld::with_modules(roster.modules());
    no_refused_writes(&tw);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    Rig {
        tw,
        oracle,
        roster,
        store: HashMap::new(),
        typed: BTreeMap::new(),
    }
}

/// Boot and sweep once, so the capital settles.
fn boot(roster: Roster) -> Rig {
    let mut rig = boot_unsettled(roster);
    sweep(&mut rig.tw, 120);
    rig
}

/// Boot, settle and stock every port, at world rate 1 (every IC instant exact).
fn stocked() -> Rig {
    stocked_on(Roster::bare())
}

/// [`stocked`] on `roster`.
fn stocked_on(roster: Roster) -> Rig {
    let mut rig = boot(roster);
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    rig
}

/// Fold every write since the first boot and reboot into the rig's roster.
fn reboot(rig: Rig) -> Rig {
    let Rig {
        mut tw,
        oracle,
        roster,
        mut store,
        typed,
    } = rig;
    fold_module_ops_into(&mut store, tw.take_module_ops());
    let mut tw = tw
        .reboot_into(roster.modules(), store.clone())
        .expect("the reboot boots");
    no_refused_writes(&tw);
    tw.engine_mut()
        .hydrate_action_state(
            typed
                .iter()
                .map(|((ns, key), value)| {
                    ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
                })
                .collect::<Vec<_>>(),
        )
        .expect("the typed state hydrates");
    with_offer_keys(&mut tw);
    install_oracle(&mut tw, &oracle);
    Rig {
        tw,
        oracle,
        roster,
        store,
        typed,
    }
}

/// Everything due `secs` on.
fn sweep(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    said(tw, &events, who)
}

fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
}

fn economy(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().expect("economy")
}

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&lane_id(), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = port_def(entry);
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let _ = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(location(tw, who), Some(room), "the teleport landed");
}

/// A fresh request/commit token per landing, across the process.
fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x5a_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Drive one queued submission the way the world task does, keeping its typed post-images
/// for a reboot.
fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    let token = next_token();
    let tw = &mut rig.tw;
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&submit.action)
            .ok_or_else(|| "that action is not currently offered".to_owned())?;
        let submission = OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: submit.parameters.clone(),
        };
        let execution = OfferExecutionContext {
            request: RequestUid::from_bytes(token),
            principal: mush_core::PrincipalId::Object { id: submit.actor },
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: submit.actor },
                    owner: submit.action.key().owner.clone(),
                    namespace: CanonicalId::new(submit.action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                IdempotencyKey::Request(RequestUid::from_bytes(token)),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|error| format!("{error:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes(token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let (ops, install) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .map_err(|error| format!("{error:?}"))?;
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                rig.typed.insert((op.ns, key), value);
            }
            ModuleWrite::Del { key } => {
                rig.typed.remove(&(op.ns, key));
            }
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    Ok(())
}

/// Every queued submission of the port's lanes, dropping other modules'; asserts none kept of
/// tacsim's is GOD's.
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from the port tick");
        }
        if owner == tacsim::NS || owner == "inventory" || owner == "market" {
            kept.push(*submit);
        }
    }
    kept
}

fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        land(rig, submit).unwrap_or_else(|error| {
            panic!(
                "{} by #{} did not land: {error}",
                submit.action.key().id,
                submit.actor.0
            )
        });
    }
}

fn land_quietly(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let _ = land(rig, submit);
    }
}

/// One port-tick fire (6 s on), its submissions drained (not landed).
fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

fn fire_quietly(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = fire(rig);
    land_quietly(rig, &queued);
    events
}

fn status(tw: &mut TestWorld) -> String {
    run(tw, GOD, "@tacsim/port status")
}

/// Fire and land the port tick until every port is stocked and listed.
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        let out = status(&mut rig.tw);
        let gated: Vec<&str> = out
            .lines()
            .filter(|line| {
                line.ends_with(']') && !line.starts_with("sky: ") && !line.starts_with("geology: ")
            })
            .collect();
        if gated.len() > 1 && gated.iter().all(|line| line.ends_with("[ready]")) {
            return;
        }
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
    }
    panic!("the ports were not stocked within 40 fires");
}

fn typed<K, V>(
    tw: &mut TestWorld,
    handle: &mush_core::StateFamilyHandle<K, V>,
    key: &K,
) -> Option<V>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let mut txn = tw
        .engine_mut()
        .action_transaction(handle)
        .expect("a transaction");
    txn.get(handle, key).expect("the family reads").cloned()
}

fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let key = mush_inventory::pool_key(EndpointRef(vault), &cid(good)).expect("a pool key");
    typed(tw, &families.pools, &key).map_or(0, |scalar| {
        u64::try_from(scalar.current()).expect("a non-negative pool")
    })
}

/// A pilot on a gateway account of their own, standing at the Registry desk.
fn pilot(rig: &mut Rig, who: &str, seed: i64) -> ObjId {
    let pilot = rig.tw.spawn_player(who);
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(
            pilot,
            AccountId::mint(&format!("{who}Acct"), seed).expect("an account id"),
        );
    }
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    pilot
}

/// An enrolled pilot (`Salt Moth`).
fn chartered(rig: &mut Rig, who: &str, seed: i64) -> (ObjId, HullRow) {
    let pilot = pilot(rig, who, seed);
    let out = run(&mut rig.tw, pilot, "+coop/enrol Salt Moth");
    assert!(out.contains("now the Salt Moth"), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row");
    (pilot, row)
}

fn hold_vault(tw: &TestWorld, row: &HullRow) -> VaultId {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    economy(tw)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id
}

fn room_of(row: &HullRow, id: &str) -> ObjId {
    ObjId(row.rooms[&cid(id)])
}

/// Seed `qty` of `good` into `vault`'s pool through `lot` (additive).
fn seed_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
    let tw = &mut rig.tw;
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    for step in 0..2 {
        let engine = tw.engine_mut();
        let prepared = {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory transaction");
            if step == 0 {
                families
                    .mint_origin(
                        &mut txn,
                        endpoint,
                        &good,
                        qty,
                        Grade::new(2).expect("grade"),
                        mush_economy::ExternalRef {
                            provider: "test".into(),
                            kind: "seed".into(),
                            id: lot.0.to_string().into(),
                            revision: None,
                        },
                        lot,
                    )
                    .map(|_| ())
                    .expect("seed mints one origin lot");
            } else {
                families
                    .dissolve(&mut txn, endpoint, lot)
                    .map(|_| ())
                    .expect("the seed lot dissolves");
            }
            txn.prepare().expect("prepares")
        };
        let (ops, install) = prepared.into_parts();
        engine.install_prepared_action(install).expect("installs");
        for op in ops {
            match op.op {
                ModuleWrite::JsonSet { key, value } => {
                    rig.typed.insert((op.ns, key), value);
                }
                ModuleWrite::Del { key } => {
                    rig.typed.remove(&(op.ns, key));
                }
                ModuleWrite::StreamAppend { .. } => {}
            }
        }
    }
}

/// `cargo_t` tonnes of metal concentrate, `tank` t and `locker` crew-days into the hold.
fn load(rig: &mut Rig, row: &HullRow, cargo_t: u64, tank: u64, locker: u64, lot: u64) {
    let vault = hold_vault(&rig.tw, row);
    if cargo_t > 0 {
        seed_pool(rig, vault, "metals", cargo_t, LotId(lot));
    }
    if tank > 0 {
        seed_pool(rig, vault, "propellant", tank, LotId(lot + 1));
    }
    if locker > 0 {
        seed_pool(rig, vault, "provisions", locker, LotId(lot + 2));
    }
}

/// Board from C01.
fn board(rig: &mut Rig, pilot: ObjId) {
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        out.contains("You cycle the lock into the Salt Moth."),
        "{out}"
    );
}

/// Drop every connection `who` holds (`orders.rs`'s), so notices queue for them.
fn disconnect(tw: &mut TestWorld, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    tw.engine_mut().step(
        mush_core::Inbound::SessionGone { actor: who },
        &mut out,
        &mut fx,
    );
}

fn world_rate(tw: &TestWorld) -> u32 {
    tw.world().world_timeline().current().rate
}

fn set_rate(tw: &mut TestWorld, rate: u32) {
    if world_rate(tw) != rate {
        let out = run(tw, GOD, &format!("@tacsim/time rate={rate}"));
        assert_eq!(world_rate(tw), rate, "{out}");
    }
}

fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_ic_now().at
}

/// Sweep to the first real second whose IC instant is at or past `target`, landing the port's
/// background submissions; the events.
fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = (ahead + rate - 1) / rate;
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_quietly(rig, &queued);
    events
}

fn voyage(tw: &TestWorld, hull: ObjId) -> Option<Voyage> {
    match store(tw).voyage_state(hull) {
        VoyageState::Present(voyage) => Some(voyage.clone()),
        VoyageState::Absent => None,
        VoyageState::Unreadable => panic!("the sailing plan is unreadable"),
    }
}

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
}

fn ticket(tw: &mut TestWorld, hull: ObjId, seq: u32) -> Option<Ticket> {
    let h = handles(tw);
    typed(tw, &h.families.tickets, &TicketId { hull, seq })
}

/// A command and the submissions it queued.
fn command(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(&mut rig.tw);
    (out, queued)
}

/// The Dray's drive as the fielded catalog holds it, at `throttle`.
fn dray(throttle: u32) -> HullDrive {
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let spec = catalog
        .specs
        .get(&cid(tacsim::definitions::content::DRAY))
        .expect("the Dray is fielded");
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let law = *catalog
        .drive_for_class(&logistics.drive.as_ref().expect("the Dray's drive").class)
        .expect("the Dray's class flies a law");
    HullDrive {
        class: logistics
            .drive
            .as_ref()
            .expect("the Dray's drive")
            .class
            .clone(),
        law,
        payload_ceiling_kg: logistics.payload_ceiling_kg,
        throttle_permille: throttle,
    }
}

/// The landing floor of `route` at `payload` kg (the reserve's throttle).
fn floor_of(route: &str, payload: u64) -> u32 {
    route_landing_floor(&lane(), &cid(route), &dray(1_000), DRY_KG, payload).expect("lifts")
}

/// File a passage with `line`, land its burn, launch it on the next fire: what the pilot
/// heard at the launch, and the ticket.
fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> (String, Ticket) {
    let (out, queued) = command(rig, pilot, line);
    assert!(out.contains("Clearance filed for "), "{line}: {out}");
    assert_eq!(queued.len(), 1, "one burn: {out}");
    assert_eq!(queued[0].actor, pilot, "the burn is the pilot's");
    let seq = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .map(|p| p.seq)
        .expect("a filed passage");
    land_all(rig, &queued);
    let t = ticket(&mut rig.tw, hull, seq).expect("the ticket");
    let events = fire_quietly(rig);
    let heard = said(&rig.tw, &events, pilot);
    (heard, t)
}

/// The arrival instant of a ticket's plan.
fn arrival_of(t: &Ticket) -> i64 {
    t.plan.depart_ic + t.plan.total_s
}

/// A chartered pilot aboard the docked *Salt Moth* at Ceres with `cargo_t` t of cargo, `tank` t
/// and `locker` crew-days, at world rate 1.
fn aboard(cargo_t: u64, tank: u64, locker: u64) -> (Rig, ObjId, HullRow) {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    load(&mut rig, &row, cargo_t, tank, locker, 9_100);
    board(&mut rig, pilot);
    (rig, pilot, row)
}

/// `@atlas/pack plan` then `@atlas/pack apply <fingerprint>` on the lane.
fn plan_and_apply(tw: &mut TestWorld) -> (String, String) {
    let plan = run(tw, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_default();
    let apply = run(tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    (plan, apply)
}

// ---------------------------------------------------------------------------
// 1. The lane
// ---------------------------------------------------------------------------

/// **The lane carries six sites** (§3.1; the brief's item 3 with lead overrides 1 and 2):
/// the places, their role, bodies and coordinates; the six vendor-less ports and their blocks;
/// the twelve routes, a hop of 3,600 s on Ceres and the engine's chord to Vesta; Ceres lists all
/// six and each lists Ceres; the Freeport Registry is Ceres's last org; sixty entries; the v8
/// shape `strip_sites` rebuilds is the published v8.
#[test]
fn sm2_the_lane_carries_six_sites() {
    let pack = lane();
    assert_eq!(pack.manifest().version(), LANE_VERSION);
    // Doc 20 cut SM.3a: v10 (the refit row and Occator's rigs; no new entry).
    assert_eq!(
        LANE_VERSION, 10,
        "lane v9 since doc 20 cut SM.2, v10 since SM.3a"
    );
    assert_eq!(
        parse_pack(LANE_JSON).expect("parses").entries().len(),
        60,
        "sixty entries (the `sol` region is the atlas's)"
    );
    assert_eq!(SITE_ROLE, "extraction", "lead override 1");
    for site in &SITES {
        let Some(PackEntry::Location(place)) = pack.entry(&cid(site.loc)) else {
            panic!("{} is a location", site.loc);
        };
        assert_eq!(place.label, site.label, "{}", site.loc);
        assert_eq!(place.role.as_str(), SITE_ROLE, "{}", site.loc);
        assert_eq!(
            place.body.as_ref().map(CanonicalId::as_str),
            Some(site.body)
        );
        assert_eq!(
            (place.lat_mdeg, place.lon_mdeg),
            (Some(site.lat), Some(site.lon)),
            "{}",
            site.loc
        );
        let def = port_def(site.entry);
        assert_eq!(def.venue.as_str(), site.venue);
        assert_eq!(def.location.as_str(), site.loc);
        assert!(def.orgs.is_empty() && def.rooms.is_empty() && def.vendors.is_empty());
        assert!(def.programme.is_none() && def.premium.is_none() && def.emergency.is_none());
        assert_eq!(def.treasury, "tacsim.treasury");
        assert_eq!(def.gallery, format!("neighborhood.{}.room.pad", site.loc));
        assert_eq!(
            def.routes,
            BTreeMap::from([(cid("ceres"), vec![cid(&format!("{}_ceres", site.venue))])])
        );
        let block_def = def.site.as_ref().expect("a site block");
        assert_eq!(block_def.name, site.label);
        assert_eq!(block_def.issuer.as_str(), "freeport_registry");
        assert_eq!(block_def.tenure.default, site.tenure);
        for letter in BLOCK_LETTERS {
            let block = block_def.block(letter);
            let want = match (site.venue, letter) {
                ("occator", 'F' | 'G' | 'J' | 'K') => (Tenure::Held, Some("occator"), 0, 0),
                ("occator", _) => (Tenure::Open, None, 1, 0),
                ("commons", _) => (Tenure::Commons, None, 0, 400),
                _ => (site.tenure, None, 0, 0),
            };
            assert_eq!(
                (
                    block.tenure,
                    block.holder.map(CanonicalId::as_str),
                    block.prospectivity,
                    block.worked_permille
                ),
                want,
                "{} block {letter}",
                site.loc
            );
        }
        for (id, from, to) in [
            (format!("ceres_{}", site.venue), "ceres_freeport", site.loc),
            (format!("{}_ceres", site.venue), site.loc, "ceres_freeport"),
        ] {
            let Some(PackEntry::Route(route)) = pack.entry(&cid(&id)) else {
                panic!("{id} is a route");
            };
            assert_eq!((route.from.as_str(), route.to.as_str()), (from, to), "{id}");
            assert_eq!(route.legs.len(), 1, "{id}");
            let want = if site.body == "ceres" {
                3_600
            } else {
                RHEASILVIA_BASE_S
            };
            assert_eq!(route.legs[0].base_s, want, "{id}");
            assert_eq!(route.drive_class.as_str(), "coast_hauler");
            assert_eq!(route.propellant, 0);
        }
    }
    let ceres = port_def("ceres_port");
    for venue in SITE_VENUES {
        assert_eq!(
            ceres.routes.get(&cid(venue)),
            Some(&vec![cid(&format!("ceres_{venue}"))]),
            "{venue}"
        );
    }
    let last = ceres.orgs.last().expect("orgs");
    assert_eq!(
        (
            last.id.as_str(),
            last.name.as_str(),
            last.capital_major,
            last.faction.as_str()
        ),
        ("freeport_registry", "Freeport Registry", 50_000, "freeport"),
        "appended last"
    );
    let Some(PackEntry::Location(freeport)) = pack.entry(&cid("ceres_freeport")) else {
        panic!("the Freeport");
    };
    assert_eq!(freeport.role.as_str(), "port", "the Freeport is no site");
    assert_eq!(
        (freeport.lat_mdeg, freeport.lon_mdeg),
        (Some(19_700), Some(240_400))
    );
    let v8 = lane_variant(8, strip_sites);
    assert_eq!(
        parse_pack(&v8).expect("v8").manifest().digest().to_hex(),
        LANE_V8_DIGEST,
        "the v8 shape is the published v8"
    );
}

// ---------------------------------------------------------------------------
// 2. The site packs
// ---------------------------------------------------------------------------

/// The shortest distance from `p` to the polygon's edges.
fn distance_to_polygon(p: (f64, f64), polygon: &[(f64, f64)]) -> f64 {
    let mut best = f64::MAX;
    for (i, a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
        best = best.min(((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt());
    }
    best
}

/// **The site packs load clean and are pinned** (§3.1; the brief's item 4): the twelve digests;
/// each region is a 2,560 m regolith square with ice and rock in its mix, relief at most 150 cm,
/// ground at 10 m; each neighborhood holds one room carrying `port.dock` over the centre cell
/// and two to four craters inside the extent, at least 300 m from the centre, 120 to 400 m
/// across, `z_range [0, 15]`, blocking sight and not movement; each validates on the
/// production roster with no blocking finding and no warning.
#[test]
fn sm2_the_site_packs_load_clean_and_are_pinned() {
    if writing() {
        for relative in site_files() {
            let path = content_dir().join(&relative);
            let raw = std::fs::read_to_string(&path).expect("the content file exists");
            let declared = parse_pack(&raw)
                .expect("the pack parses")
                .manifest()
                .digest()
                .to_hex();
            let unsigned = raw.replace(&declared, UNSIGNED);
            std::fs::write(&path, sign(&unsigned)).expect("the content file is writable");
        }
        return;
    }
    let mut report = Vec::new();
    for ((name, json), (pack, pinned)) in SITE_SOURCES.iter().zip(PINNED) {
        let parsed = parse_pack(json).expect(name);
        assert_eq!(parsed.manifest().id().as_str(), pack);
        assert_eq!(parsed.manifest().version(), 1);
        let actual = parsed.computed_digest().expect("digests").to_hex();
        if actual != pinned {
            report.push(format!("    ({pack:?}, {actual:?}),"));
        }
    }
    assert!(
        report.is_empty(),
        "site pack digests moved; if the edit was reviewed, pin them:\n{}",
        report.join("\n")
    );
    for (name, json) in SITE_SOURCES {
        let pack = parse_pack(json).expect(name);
        assert_eq!(
            pack.computed_digest().expect("digests").to_hex(),
            pack.manifest().digest().to_hex(),
            "{name}: the digest field must be re-derived, never hand-edited"
        );
        let doc: Value = serde_json::from_str(json).expect(name);
        let entries = doc["entries"].as_object().expect("entries");
        if name.starts_with("region.") {
            let region = &entries[name]["def"];
            assert_eq!(region["kind"], "surface", "{name}");
            assert_eq!(
                region["extent"],
                serde_json::json!([[0, 0], [2560, 2560]]),
                "{name}"
            );
            assert_eq!(region["ground_resolution_cm"], 1000, "{name}");
            let bg = &region["background"];
            assert_eq!(bg["surface"], "regolith", "{name}");
            let mix: Vec<&str> = bg["mix"]
                .as_array()
                .expect("a mix")
                .iter()
                .filter_map(|m| m[0].as_str())
                .collect();
            assert_eq!(mix, vec!["ice", "rock"], "{name}");
            assert!(bg["relief_cm"].as_i64().expect("relief") <= 150, "{name}");
            continue;
        }
        let rooms: Vec<&Value> = entries.values().filter(|e| e["kind"] == "room").collect();
        assert_eq!(rooms.len(), 1, "{name}: one pad");
        let pad = &rooms[0]["def"];
        assert_eq!(pad["markers"], serde_json::json!(["port.dock"]), "{name}");
        let rect = &pad["footprint"]["rect"];
        let corner = |i: usize, j: usize| rect[i][j].as_f64().expect("a coordinate");
        assert!(
            corner(0, 0) <= 1_280.0
                && 1_280.0 <= corner(1, 0)
                && corner(0, 1) <= 1_280.0
                && 1_280.0 <= corner(1, 1),
            "{name}: the pad covers the centre"
        );
        let craters: Vec<&Value> = entries
            .values()
            .filter(|e| e["kind"] == "feature" && e["def"]["feature_kind"] == "crater")
            .collect();
        assert!(
            (2..=4).contains(&craters.len()),
            "{name}: {}",
            craters.len()
        );
        for crater in craters {
            let polygon: Vec<(f64, f64)> = crater["def"]["polygon"]
                .as_array()
                .expect("a polygon")
                .iter()
                .map(|p| (p[0].as_f64().expect("x"), p[1].as_f64().expect("y")))
                .collect();
            assert!((8..=12).contains(&polygon.len()), "{name}");
            assert!(
                polygon
                    .iter()
                    .all(|(x, y)| (0.0..=2_560.0).contains(x) && (0.0..=2_560.0).contains(y)),
                "{name}: inside the extent"
            );
            assert!(
                distance_to_polygon((1_280.0, 1_280.0), &polygon) >= 300.0,
                "{name}: a crater within 300 m of the pad"
            );
            // The brief's item 4 (the SM.2 fixer, the lead's item 5): 120 to 400 m across
            // (its widest chord, vertex to vertex), `z_range [0, 15]`, blocking sight and
            // not movement.
            let across = polygon
                .iter()
                .flat_map(|a| {
                    polygon
                        .iter()
                        .map(move |b| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt())
                })
                .fold(0.0_f64, f64::max);
            assert!(
                (120.0..=400.0).contains(&across),
                "{name}: a crater {across} m across"
            );
            assert_eq!(crater["def"]["surface"], "ground", "{name}");
            assert_eq!(
                crater["def"]["z_range"],
                serde_json::json!([0, 15]),
                "{name}"
            );
            assert_eq!(
                crater["def"]["blocks"],
                serde_json::json!({ "movement": false, "sight": true }),
                "{name}"
            );
        }
    }
    // Validated on the production roster: nothing blocks and nothing warns.
    let rig = boot_unsettled(Roster::bare());
    let w = rig.tw.world();
    let atlas = w.module_state::<AtlasState>().expect("atlas");
    let content = w.module_state::<ContentState>().expect("content");
    for (name, json) in SITE_SOURCES {
        let pack = mush_content::load_pack(json).expect(name);
        let cx = ValidateContext::for_pack(w, atlas, content, &pack);
        let noisy: Vec<String> = validate(&pack, &cx)
            .iter()
            .filter(|f| f.blocks() || f.severity == mush_atlas::Severity::Warning)
            .map(mush_atlas::Finding::line)
            .collect();
        assert!(noisy.is_empty(), "{name}: {noisy:#?}");
    }
}

// ---------------------------------------------------------------------------
// 3. The validator
// ---------------------------------------------------------------------------

/// The lane with `edit`, decoded: the refusal's text, or "accepted".
fn decoded(edit: impl FnOnce(&mut Value)) -> String {
    let json = lane_variant(LANE_VERSION, edit);
    match parse_pack(&json) {
        Err(error) => error.to_string(),
        Ok(pack) => match rules_pack_of(pack) {
            Ok(_) => "accepted".to_owned(),
            Err(error) => error.to_string(),
        },
    }
}

/// An edit to the lane's JSON.
type Edit = Box<dyn FnOnce(&mut Value)>;

/// **The validator refuses what a site may not author** (§3.1, §9.5; the brief's items 1 and
/// 2), each refusal naming its rule.
#[test]
fn sm2_the_validator_refuses_what_a_site_may_not_author() {
    assert_eq!(decoded(|_| {}), "accepted");
    let blocks = |rows: Value| -> Edit {
        Box::new(move |doc: &mut Value| {
            doc["entries"]["ahuna_port"]["def"]["site"]["blocks"] = rows;
        })
    };
    let site_field = |field: &'static str, value: Value| -> Edit {
        Box::new(move |doc: &mut Value| {
            doc["entries"]["ahuna_port"]["def"]["site"][field] = value;
        })
    };
    let without = |entry: &'static str, field: &'static str| -> Edit {
        Box::new(move |doc: &mut Value| {
            doc["entries"][entry]["def"]
                .as_object_mut()
                .expect("an entry")
                .remove(field);
        })
    };
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "orgs on a site",
            Box::new(|doc| {
                doc["entries"]["ahuna_port"]["def"]["orgs"] = serde_json::json!([
                    { "id": "ahuna_co", "name": "Ahuna Co", "capital_major": 1, "faction": "freeport" }
                ]);
            }),
            "site: a site authors no orgs, rooms, programme, vendors, premium or emergency",
        ),
        (
            "rooms on a site",
            Box::new(|doc| {
                doc["entries"]["ahuna_port"]["def"]["rooms"] =
                    serde_json::json!({ "registry": "neighborhood.ahuna_flank.room.pad" });
            }),
            "site: a site authors no orgs, rooms, programme, vendors, premium or emergency",
        ),
        (
            "a held row without a holder",
            blocks(serde_json::json!({ "F": { "tenure": "held" } })),
            "site: block F: a held block names its holder, and only a held block does",
        ),
        (
            "a holder on an open row",
            blocks(serde_json::json!({ "F": { "tenure": "open", "holder": "occator" } })),
            "site: block F: a held block names its holder, and only a held block does",
        ),
        (
            "prospectivity 4",
            blocks(serde_json::json!({ "A": { "prospectivity": 4 } })),
            "site: block A: prospectivity is -3 to 3, not 4",
        ),
        (
            "worked 1001",
            blocks(serde_json::json!({ "A": { "worked_permille": 1001 } })),
            "site: block A: worked_permille is 0 to 1000, not 1001",
        ),
        (
            "letter Q",
            blocks(serde_json::json!({ "Q": { "prospectivity": 1 } })),
            "site: block Q is not a block letter A to P",
        ),
        (
            "zone frontier",
            site_field("zone", serde_json::json!("frontier")),
            "frontier",
        ),
        (
            "default held",
            Box::new(|doc| {
                doc["entries"]["ahuna_port"]["def"]["site"]["tenure"]["default"] =
                    serde_json::json!("held");
            }),
            "site: the default tenure is commons or open; a held block names its holder",
        ),
        (
            "a name that is not the label",
            site_field("name", serde_json::json!("Ahuna Slope")),
            "site: name `Ahuna Slope` is not its location's label `Ahuna Flank`",
        ),
        (
            "an unknown issuer",
            site_field("issuer", serde_json::json!("nobody")),
            "site: issuer nobody is not an org of any port of this pack",
        ),
        (
            "an unknown holder",
            blocks(serde_json::json!({ "F": { "tenure": "held", "holder": "nobody" } })),
            "site: block F: holder nobody is not an org of any port of this pack",
        ),
        (
            "an extraction place under a station",
            Box::new(|doc| {
                doc["entries"]["ceres_freeport"]["def"]["role"] = serde_json::json!("extraction");
            }),
            "location ceres_freeport has role extraction, which only a site's port may stand on",
        ),
        (
            "a site on a station's role",
            Box::new(|doc| {
                doc["entries"]["ahuna_flank"]["def"]["role"] = serde_json::json!("port");
            }),
            "site: location ahuna_flank has role port, not extraction",
        ),
        (
            "a site body without geology",
            without("vesta", "geology"),
            "site: body vesta authors no geology",
        ),
        (
            "a site body without surface_mg",
            without("ceres", "surface_mg"),
            "site: body ceres authors no surface_mg",
        ),
        (
            "a geology without its radius",
            without("ceres", "radius_km"),
            "a body's geology needs its radius_km",
        ),
        (
            "a surface figure out of range",
            Box::new(|doc| {
                doc["entries"]["pallas"]["def"]["surface_mg"] = serde_json::json!(10_001);
            }),
            "a body's surface_mg is 1 to 10000 milli-g",
        ),
        (
            "latitude 90001",
            Box::new(|doc| {
                doc["entries"]["ahuna_flank"]["def"]["lat_mdeg"] = serde_json::json!(90_001);
            }),
            "place ahuna_flank: lat_mdeg is -90000 to 90000 milli-degrees, not 90001",
        ),
        (
            "longitude 360000",
            Box::new(|doc| {
                doc["entries"]["ahuna_flank"]["def"]["lon_mdeg"] = serde_json::json!(360_000);
            }),
            "place ahuna_flank: lon_mdeg is 0 to 359999 milli-degrees east, not 360000",
        ),
        (
            "one coordinate alone",
            without("ahuna_flank", "lon_mdeg"),
            "place ahuna_flank: a place authors both lat_mdeg and lon_mdeg, or neither",
        ),
        (
            "coordinates without a body",
            Box::new(|doc| {
                doc["entries"].as_object_mut().expect("entries").insert(
                    "nowhere".to_owned(),
                    serde_json::json!({ "kind": "location", "def": {
                        "label": "Nowhere", "role": "depot", "lat_mdeg": 0, "lon_mdeg": 0 } }),
                );
            }),
            "place nowhere: a place with coordinates stands on a body",
        ),
    ];
    for (why, edit, rule) in cases {
        let refused = decoded(edit);
        assert!(refused.contains(rule), "{why}: {refused}");
    }
}

// ---------------------------------------------------------------------------
// 4. The ground
// ---------------------------------------------------------------------------

/// A lane body's ground, from its own `BodyDef`.
fn body_ground(pack: &RulesPack, body: &str) -> BodyGeology {
    match pack.entry(&cid(body)) {
        Some(PackEntry::Body(def)) => BodyGeology::new(
            cid(body),
            def.radius_km.expect("a radius"),
            def.geology.clone().expect("a table"),
        )
        .expect("a body"),
        other => panic!("{body} is a body: {other:?}"),
    }
}

/// The location entry of a site place.
fn place(pack: &RulesPack, loc: &str) -> tacsim::pack::LocationDef {
    match pack.entry(&cid(loc)) {
        Some(PackEntry::Location(def)) => def.clone(),
        other => panic!("{loc} is a location: {other:?}"),
    }
}

/// **The sites' geology builds and no two sites overlap** (§3.1, §3.2; decision 11): under the
/// fixed test key every site's table builds from its own lane entry; per block and family at
/// most 64 ore pockets; the same-body sites' 64-pocket squares are pairwise disjoint; Occator
/// Rim's twelve outer blocks shift +1 and F, G, J, K read held by Occator; Freeport Commons's
/// sixteen rows read worked 400.
#[test]
fn sm2_the_sites_geology_builds_and_no_two_sites_overlap() {
    let pack = lane();
    let k = mac(KEY_1);
    let mut squares: Vec<(&str, &str, (i64, i64), i64)> = Vec::new();
    for site in &SITES {
        let ground = body_ground(&pack, site.body);
        let def = port_def(site.entry);
        let block_def = def.site.as_ref().expect("a site");
        let spec = geology_spec(block_def, &place(&pack, site.loc), &ground).expect("a spec");
        assert_eq!(spec.body.as_str(), site.body);
        let geology = site_geology(&k, 1, &ground, &spec).expect("the site builds");
        for by in 0..4_i64 {
            for bx in 0..4_i64 {
                let mut counts = [0_u32; 4];
                for py in by * 16..by * 16 + 16 {
                    for px in bx * 16..bx * 16 + 16 {
                        let row = geology.pocket(px, py).expect("a pocket");
                        for (n, class) in row.classes.iter().enumerate() {
                            if *class != Word::Barren {
                                counts[n] += 1;
                            }
                        }
                    }
                }
                assert!(
                    counts.iter().all(|c| *c <= 64),
                    "{} block {bx},{by}: {counts:?}",
                    site.loc
                );
            }
        }
        squares.push((site.loc, site.body, spec.origin, ground.lattice().columns));
        if site.loc == "occator_rim" {
            let mut want = [1_i8; 16];
            for slot in [5, 6, 9, 10] {
                want[slot] = 0;
            }
            assert_eq!(spec.shifts, want, "Occator's outer twelve +1");
            for letter in ['F', 'G', 'J', 'K'] {
                let block = block_def.block(letter);
                assert_eq!(block.tenure, Tenure::Held);
                assert_eq!(block.holder.map(CanonicalId::as_str), Some("occator"));
            }
        } else {
            assert_eq!(spec.shifts, [0_i8; 16], "{}", site.loc);
        }
        if site.loc == "freeport_commons" {
            assert!(
                BLOCK_LETTERS
                    .iter()
                    .all(|l| block_def.block(*l).worked_permille == 400)
            );
        }
    }
    for (i, a) in squares.iter().enumerate() {
        for b in squares.iter().skip(i + 1) {
            if a.1 != b.1 {
                continue;
            }
            let columns = a.3;
            let dx = (a.2.0 - b.2.0).rem_euclid(columns);
            let dx = dx.min(columns - dx);
            let dy = (a.2.1 - b.2.1).abs();
            assert!(dx >= 64 || dy >= 64, "{} and {} overlap", a.0, b.0);
        }
    }
}

fn cached(tw: &TestWorld) -> usize {
    tw.world()
        .module_state::<tacsim::surface::GeologyState>()
        .expect("the geology slot")
        .sites()
        .len()
}

/// A site's 256 × 256 window over its own region (the region pack's id is the record's pack and
/// entry both).
fn site_frame(loc: &str) -> mush_atlas::window::WindowFrame {
    let region = format!("region.{loc}");
    mush_atlas::window::WindowFrame {
        region: mush_atlas::RecordKey::new(cid(&region), cid(&region)),
        surfaces: vec![mush_atlas::SurfaceRef::Ground],
        bbox: mush_atlas::RectCm {
            min: mush_atlas::PointCm { x: 0, y: 0 },
            max: mush_atlas::PointCm {
                x: 256_000,
                y: 256_000,
            },
        },
        cell_cm: 1_000,
    }
}

/// A site's spec from its own lane entry.
fn spec_of(
    pack: &RulesPack,
    entry: &str,
    loc: &str,
    body: &str,
) -> tacsim::surface::SiteGeologySpec {
    let def = port_def(entry);
    geology_spec(
        def.site.as_ref().expect("a site"),
        &place(pack, loc),
        &body_ground(pack, body),
    )
    .expect("a spec")
}

/// **The lane's tables are §3.2's** (decision 3): the lane's Ceres and Vesta tables equal SM.1's
/// reviewed test tables, so SM.1's property tests cover them; a world booted on the production
/// roster has exactly Ceres and Vesta installed, and the boot pass's install run again leaves
/// each body's site cache standing (an equal table is never reinstalled).
#[test]
fn sm2_the_lane_tables_are_section_3_2s() {
    let pack = lane();
    let table = |json: &str| {
        GeologyTable::from_wire(&serde_json::from_str::<GeologyTableWire>(json).expect("wire"))
            .expect("a table")
    };
    for (body, json) in [("ceres", CERES_TABLE), ("vesta", VESTA_TABLE)] {
        let Some(PackEntry::Body(def)) = pack.entry(&cid(body)) else {
            panic!("{body}");
        };
        assert_eq!(def.geology.as_ref(), Some(&table(json)), "{body}");
    }
    let mut tw = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), HashMap::new(), true);
    assert_eq!(
        installed_bodies(tw.world()),
        vec![cid("ceres"), cid("vesta")]
    );
    let frame = site_frame("ahuna_flank");
    let spec = spec_of(&pack, "ahuna_port", "ahuna_flank", "ceres");
    let built = site_ground_sync(tw.world_mut(), &frame, &spec).expect("the site builds");
    assert_eq!(cached(&tw), 1);
    let bodies = tacsim::surface::lane_geologies(&pack);
    assert_eq!(bodies.len(), 2, "the two bodies with a table");
    assert_eq!(
        tacsim::surface::install_lane_geology(tw.world_mut(), bodies),
        0
    );
    assert_eq!(cached(&tw), 1, "an equal table keeps the site cache");
    let again = site_ground_sync(tw.world_mut(), &frame, &spec).expect("a hit");
    assert!(Arc::ptr_eq(&built, &again), "the second call is a hit");

    // A restart (reviewer T2): the lane is latched and never re-applied, so the boot pass
    // (lib.rs, after the key) is the only installer — the tables stand and the survey reads.
    let folded = mush_testkit::fold_module_ops(tw.take_module_ops());
    let mut rebooted = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), folded, true);
    assert_eq!(
        installed_bodies(rebooted.world()),
        vec![cid("ceres"), cid("vesta")],
        "the boot pass installs the lane's tables on a restart"
    );
    let chart = run(&mut rebooted, GOD, "+chart");
    let lines: Vec<&str> = chart.lines().collect();
    let (_, survey, _) = under(&lines, "Ahuna Flank");
    assert!(
        survey.starts_with("    Registry survey: ") && survey != SURVEY_UNAVAILABLE,
        "{chart}"
    );
}

/// **A re-apply keeps each body's site cache** (reviewer T2; the skip-equal rule on the
/// apply path): on a keyed world whose Ahuna ground is cached, an update of the lane that
/// touches a site port (Kerwan's block A) runs the `PortApplier`'s part (g), whose body
/// tables equal the installed ones, so nothing is reinstalled and the cached site is still
/// a hit.
#[test]
fn sm2_a_site_re_apply_keeps_the_site_cache() {
    let next = lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["kerwan_port"]["def"]["site"]["blocks"] =
            serde_json::json!({ "A": { "prospectivity": 1 } });
    });
    let roster = Roster {
        shelf: Some(vec![
            mush_content::load_pack(&next).expect("the variant verifies"),
        ]),
        extra_lane: Some(next),
        ..Roster::default()
    };
    let (world, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, roster.modules())
        .expect("the roster composes");
    assert!(tacsim::surface::install_geology(
        &mut engine.world,
        GeologyBoot::Key(secret(KEY_1))
    ));
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .expect("the world boots");
    let mut tw = TestWorld::from_engine(engine);
    no_refused_writes(&tw);
    with_offer_keys(&mut tw);
    assert_eq!(
        installed_bodies(tw.world()),
        vec![cid("ceres"), cid("vesta")]
    );
    let pack = lane();
    let frame = site_frame("ahuna_flank");
    let spec = spec_of(&pack, "ahuna_port", "ahuna_flank", "ceres");
    let built = site_ground_sync(tw.world_mut(), &frame, &spec).expect("the site builds");
    assert_eq!(cached(&tw), 1);
    let (plan, apply) = plan_and_apply(&mut tw);
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{}: 0 created, 1 updated",
            LANE_VERSION + 1
        )),
        "{plan}\n{apply}"
    );
    assert_eq!(cached(&tw), 1, "part (g) reinstalled no equal table");
    let again = site_ground_sync(tw.world_mut(), &frame, &spec).expect("a hit");
    assert!(
        Arc::ptr_eq(&built, &again),
        "the cached site survived the apply"
    );
}

/// **The site door reads the site's own region** (lead override 4, SM.1's residual SM1-A7): the
/// Vesta region with a Ceres site's spec is refused, naming both bodies, and nothing is cached;
/// with Rheasilvia's own spec it builds its 256 × 256 ground.
#[test]
fn sm2_the_site_door_refuses_a_spec_for_another_body() {
    let pack = lane();
    let mut tw = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), HashMap::new(), true);
    let frame = site_frame("rheasilvia_commons");
    let ahuna = spec_of(&pack, "ahuna_port", "ahuna_flank", "ceres");
    let refused = site_ground_sync(tw.world_mut(), &frame, &ahuna).expect_err("refused");
    assert!(
        refused.starts_with("the site names ceres, but region ")
            && refused.ends_with(" is on vesta"),
        "{refused}"
    );
    assert_eq!(cached(&tw), 0, "a refusal caches nothing");
    let rhea = spec_of(&pack, "rheasilvia_port", "rheasilvia_commons", "vesta");
    let ground = site_ground_sync(tw.world_mut(), &frame, &rhea).expect("Rheasilvia builds");
    assert_eq!(ground.geology.body().as_str(), "vesta");
    assert_eq!(cached(&tw), 1);
}

// ---------------------------------------------------------------------------
// 5. The lay-up and readiness
// ---------------------------------------------------------------------------

/// A site port's readiness `Site` line.
fn site_line(report: &tacsim::port::Readiness, entry: &str) -> tacsim::port::ReadinessLine {
    report
        .lines
        .iter()
        .find(|l| {
            l.binding == Binding::Site && l.port.as_ref().map(CanonicalId::as_str) == Some(entry)
        })
        .cloned()
        .expect("a site line")
}

/// **Sixteen place objects and a clerk a site** (acceptance 7; §3.4; the brief's item 8): per
/// site the map's row has sixteen letters and a clerk, each live, `DARK`, in the host room; the
/// readiness `Site` line is Ready; the applier submitted nothing.
#[test]
fn sm2_each_site_lays_up_sixteen_places_and_a_clerk() {
    let rig = boot_unsettled(Roster::bare());
    assert_eq!(
        rig.tw.engine().queued_authoritative_actors().count(),
        0,
        "the content pass queued an authoritative submission"
    );
    let report = tacsim::port::readiness(rig.tw.world());
    for entry in SITE_PORTS {
        let map = port_map(&rig.tw, entry);
        let host = ObjId(map.host.expect("a host"));
        let row = map.site.as_ref().expect("a site row");
        let letters: Vec<String> = BLOCK_LETTERS.iter().map(char::to_string).collect();
        assert_eq!(
            row.blocks.keys().cloned().collect::<Vec<_>>(),
            letters,
            "{entry}"
        );
        let def = port_def(entry);
        let name = def.site.as_ref().expect("a site").name.clone();
        let mut things: Vec<(u32, String)> = row
            .blocks
            .iter()
            .map(|(letter, thing)| (*thing, format!("{name} block {letter}")))
            .collect();
        things.push((row.clerk.expect("a clerk"), format!("{name} site clerk")));
        for (thing, want) in things {
            let obj = rig.tw.world().get(ObjId(thing)).expect("live");
            assert_eq!(obj.name, want);
            assert!(obj.flags.contains(Flags::DARK), "{want}");
            assert!(!obj.flags.contains(Flags::GOING), "{want}");
            assert_eq!(obj.location, Some(host), "{want} stands in the host room");
        }
        let line = site_line(&report, entry);
        assert_eq!(
            (line.state, line.text.as_str()),
            (
                LineState::Ready,
                format!("{entry} site: 16 block places and the site clerk laid up.").as_str()
            )
        );
        assert!(report.sites.contains(&cid(entry)), "{entry}");
    }
}

/// **A site's update re-lays nothing; a gone place refuses** (the brief's item 8): a v10 variant
/// changing one block's prospectivity applies as an update and creates no Thing; on a second
/// world a recorded block Thing gone reads Missing on the `Site` line, never shuts the desk, and
/// makes the apply refuse naming it.
#[test]
fn sm2_a_site_update_lays_up_nothing_and_a_gone_place_refuses() {
    let next = lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["kerwan_port"]["def"]["site"]["blocks"] =
            serde_json::json!({ "A": { "prospectivity": 1 } });
    });
    let roster = Roster {
        shelf: Some(vec![
            mush_content::load_pack(&next).expect("the variant verifies"),
        ]),
        extra_lane: Some(next),
        ..Roster::default()
    };
    let mut rig = boot(roster.clone());
    let objects = |tw: &TestWorld| tw.world().iter().count();
    let before = objects(&rig.tw);
    let kerwan = port_map(&rig.tw, "kerwan_port");
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{}: 0 created, 1 updated",
            LANE_VERSION + 1
        )),
        "{plan}\n{apply}"
    );
    assert_eq!(objects(&rig.tw), before, "an update makes no Thing");
    assert_eq!(port_map(&rig.tw, "kerwan_port").site, kerwan.site);

    // A recorded block place flagged GOING, on a stocked world (every station line ready, so
    // the enrolment gate reads the site's line itself — reviewer T5).
    let mut rig = stocked_on(roster.clone());
    let gone = port_map(&rig.tw, "kerwan_port").site.expect("a row").blocks["C"];
    rig.tw
        .world_mut()
        .set_flag(ObjId(gone), Flags::GOING, true)
        .expect("flagged");
    let report = tacsim::port::readiness(rig.tw.world());
    let line = site_line(&report, "kerwan_port");
    assert_eq!(
        (line.state, line.text.as_str()),
        (
            LineState::Missing,
            "kerwan_port site: 15 of 16 block places and the site clerk laid up."
        )
    );
    assert!(!report.is_ready(), "the summary counts it");
    assert!(
        tacsim::port::enrol_gate(&report).is_ok(),
        "a site's line never shuts the desk: {:?}",
        tacsim::port::enrol_gate(&report).err()
    );
    let refusal = format!(
        "kerwan_port: the site's block place C #{gone} in the port map is gone; it is never \
         re-created — repair the map by hand"
    );
    assert_refused(&mut rig.tw, &refusal);

    // The site clerk destroyed outright (World::destroy, then swept to a tombstone).
    let mut rig = stocked_on(roster);
    let clerk = port_map(&rig.tw, "kerwan_port")
        .site
        .and_then(|row| row.clerk)
        .expect("a clerk");
    rig.tw.world_mut().destroy(ObjId(clerk)).expect("destroyed");
    let _ = rig.tw.world_mut().sweep();
    let report = tacsim::port::readiness(rig.tw.world());
    let line = site_line(&report, "kerwan_port");
    assert_eq!(
        (line.state, line.text.as_str()),
        (
            LineState::Missing,
            "kerwan_port site: 16 of 16 block places and no site clerk laid up."
        )
    );
    assert!(tacsim::port::enrol_gate(&report).is_ok());
    assert_refused(
        &mut rig.tw,
        &format!(
            "kerwan_port: the site clerk #{clerk} in the port map is gone; it is never \
             re-created — repair the map by hand"
        ),
    );
}

/// The lane's update to `LANE_VERSION + 1` refuses with `refusal` and applies nothing: no
/// `Applied` line, the latch still at the shipped version.
fn assert_refused(tw: &mut TestWorld, refusal: &str) {
    let (plan, apply) = plan_and_apply(tw);
    assert!(
        plan.contains(refusal) || apply.contains(refusal),
        "{refusal}\n{plan}\n{apply}"
    );
    assert!(!apply.contains("Applied "), "{plan}\n{apply}");
    assert_eq!(
        store(tw).applied_in(tw.world(), &lane_id()),
        Some(LANE_VERSION),
        "the latch did not move"
    );
}

// ---------------------------------------------------------------------------
// 6. Landing, the stay and the lift
// ---------------------------------------------------------------------------

/// **The hop settles on the pad and lifts home** (acceptance 1's ticket and acceptance 3): a
/// full tank, a 120 crew-day locker, 6 t of cargo and one soul plan and file the hop to Ahuna at
/// `⌊3,600 × 267,580 / 181,000⌋` = 5,322 s and one tonne, with the leg home held back; the hull
/// settles on the pad, reads settled, keeps its crew aboard and lifts home with `+ship/lift`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm2_a_hop_settles_on_the_pad_and_lifts_home() {
    let (mut rig, pilot, row) = aboard(6, 80, 120);
    let hull = ObjId(row.hull);
    let payload: u64 = 6_000 + 80_000 + 480 + 100;

    // +ship/plan ahuna: a tonne, the leg home, no Throttle: line.
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    let lines: Vec<&str> = plan.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("Plan for Ahuna Flank:"),
        "{plan}"
    );
    assert!(lines.contains(&"Payload 86.6 t of 181 t."), "{plan}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Burn 1 t (tank 80 t; 79 t left). ")),
        "{plan}"
    );
    assert!(
        lines.contains(&"Stores 1 crew-days (locker 120; 119 left)."),
        "{plan}"
    );
    let home = SITE_HOME
        .replace("{port}", "Ahuna Flank")
        .replace("{burn}", "1")
        .replace("{days}", "1")
        .replace(
            "{throttle}",
            &floor_of("ahuna_ceres", payload - 1_000 - 4).to_string(),
        );
    assert!(lines.contains(&home.as_str()), "{home}\n{plan}");
    assert!(!lines.iter().any(|l| l.starts_with("Throttle:")), "{plan}");

    // +ship/undock ahuna: filed with the arrival row, burned, launched.
    let (heard, t) = file_and_launch(&mut rig, pilot, hull, "+ship/undock ahuna");
    let formula = i64::try_from(3_600 * (181_000 + payload) / 181_000).expect("fits");
    assert_eq!(t.plan.total_s, formula, "the binding assertion");
    assert_eq!(t.plan.total_s, 5_322);
    assert_eq!((t.plan.burn_t, t.plan.crew_days), (1, 1));
    assert_eq!(t.plan.legs[0].ascent_mps, 360);
    assert_eq!(t.plan.landed_crew_days, 0);
    assert_eq!(t.payload_kg, payload);
    assert!(
        heard.contains("The Salt Moth's lock seals. Berth clamps release; the drive lights."),
        "{heard}"
    );
    let arrival = voyage(&rig.tw, hull)
        .and_then(|v| v.arrival)
        .expect("the launched passage's arrival row");
    assert_eq!(
        (arrival.locker, arrival.home_crew_days, arrival.souls),
        (119, 1, 1)
    );

    // The arrival: settled, the stores line, the berth.
    let events = to_ic(&mut rig, arrival_of(&t));
    let heard = said(&rig.tw, &events, pilot);
    let settles = SETTLES_PREFIX.to_owned() + " Stores 119 crew-days: ";
    assert!(heard.contains(&settles), "{heard}");
    assert!(heard.contains("here before short rations."), "{heard}");
    match berth_of(rig.tw.world(), hull) {
        Berth::Docked { def, .. } => assert_eq!(def.venue.as_str(), "ahuna"),
        other => panic!("not on the pad: {other:?}"),
    }
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(
        ship.contains("Salt Moth (Dray): settled on the pad at Ahuna Flank."),
        "{ship}"
    );
    assert!(ship.contains("Stores 119 crew-days: "), "{ship}");
    let look = run(&mut rig.tw, pilot, "look");
    assert!(
        look.contains(&format!(
            "Aboard the Salt Moth ({}), settled on the pad at Ahuna Flank.",
            row.mark
        )),
        "{look}"
    );
    // Nobody steps out onto a pad.
    assert_eq!(location(&rig.tw, pilot), Some(room_of(&row, "airlock")));
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert_eq!(
        out,
        LEAVE_PAD
            .replace("{name}", "Salt Moth")
            .replace("{port}", "Ahuna Flank")
    );
    assert_eq!(location(&rig.tw, pilot), Some(room_of(&row, "airlock")));
    assert_eq!(run(&mut rig.tw, pilot, "+ship/lift"), LIFT_USAGE);

    // +ship/lift ceres: the pad drops away; she docks at Ceres.
    let (heard, back) = file_and_launch(&mut rig, pilot, hull, "+ship/lift ceres");
    assert!(
        heard.contains(&LIFT_SEALS.replace("{name}", "Salt Moth")),
        "{heard}"
    );
    assert_eq!(back.plan.route.as_str(), "ahuna_ceres");
    assert_eq!(back.plan.legs[0].ascent_mps, 360);
    let events = to_ic(&mut rig, arrival_of(&back));
    let heard = said(&rig.tw, &events, pilot);
    assert!(
        heard.contains("Salt Moth docks at Ceres Freeport."),
        "{heard}"
    );
    match berth_of(rig.tw.world(), hull) {
        Berth::Docked { def, .. } => assert_eq!(def.venue.as_str(), "ceres"),
        other => panic!("not docked at Ceres: {other:?}"),
    }
}

/// `SETTLES` for the Salt Moth at Ahuna Flank.
const SETTLES_PREFIX: &str = "Salt Moth settles onto the pad at Ahuna Flank.";

/// **A hull landed ten days still lifts** (acceptance 4; Q-20-7, lead ruling 5): one soul, a
/// locker of six crew-days at Ceres (the hop out and the leg home need two), so she arrives with
/// five; a day on the pad reads four (the charge a lift would make, the raw pool still five); ten
/// days read short rations at one; the lift charges four and burns the locker to nothing; she
/// docks at Ceres.
#[test]
fn sm2_a_hull_landed_ten_days_still_lifts() {
    let (mut rig, pilot, row) = aboard(0, 80, 6);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let (_, t) = file_and_launch(&mut rig, pilot, hull, "+ship/undock ahuna");
    let since = arrival_of(&t);
    // Off watch at the landing: the settled notice is queued with the stores line (decision 8:
    // the once-per-stay short-rations notice), read at the landing instant.
    disconnect(&mut rig.tw, pilot);
    let _ = to_ic(&mut rig, since);
    let notices = tacsim::port_tick::pending_notices(rig.tw.world(), pilot);
    let settled = SETTLED_NOTICE
        .replace("{hhmm} UTC ", "")
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Ahuna Flank");
    assert!(
        notices.iter().any(|n| n.ends_with(&format!(
            "{settled} Stores 5 crew-days: 4 d 0 h here before short rations."
        )) && n.contains(" UTC Salt Moth settled")),
        "{notices:#?}"
    );
    let _ = rig.tw.connect(pilot);
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 5, "6 - 1");
    let _ = to_ic(&mut rig, since + 86_400);
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(ship.contains("Stores 4 crew-days: "), "{ship}");
    assert_eq!(
        pool(&mut rig.tw, vault, "provisions"),
        5,
        "nothing debits on the pad"
    );
    let _ = to_ic(&mut rig, since + 10 * 86_400);
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(
        ship.contains(
            "Stores 1 crew-days: on short rations here; the stores for the leg home are kept back."
        ),
        "{ship}"
    );
    // `+ship/plan ceres` carries the charge the lift would make (reviewer T7): the leg's
    // crew-day and min(10, 5 - 1) landed, and the effective stores line.
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    let lines: Vec<&str> = plan.lines().collect();
    assert!(
        lines.contains(&"Stores 5 crew-days (locker 5; 0 left)."),
        "{plan}"
    );
    assert!(
        lines.contains(
            &"Stores 1 crew-days: on short rations here; the stores for the leg home are kept back."
        ),
        "{plan}"
    );
    // A queued depart names its hour on the pad (`SHIP_SETTLED_DEPARTS`,
    // `STATUS_SETTLED_DEPARTS`), and is cleared again.
    let out = run(&mut rig.tw, pilot, "+ship/order depart ceres at=+2h");
    assert!(
        out.contains("Order 1 filed: depart for Ceres Freeport at "),
        "{out}"
    );
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(
        ship.contains("Salt Moth (Dray): settled on the pad at Ahuna Flank; departs ")
            && ship.contains(" UTC (in 2 h 0 m)."),
        "{ship}"
    );
    let look = run(&mut rig.tw, pilot, "look");
    assert!(
        look.contains(&format!(
            "Aboard the Salt Moth ({}), settled on the pad at Ahuna Flank; departs ",
            row.mark
        )),
        "{look}"
    );
    let _ = run(&mut rig.tw, pilot, "+ship/order clear");
    let (_, back) = file_and_launch(&mut rig, pilot, hull, "+ship/lift ceres");
    assert_eq!(
        (back.plan.landed_crew_days, back.plan.crew_days),
        (4, 5),
        "min(10, 5 - 1) charged at the lift"
    );
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 0);
    let events = to_ic(&mut rig, arrival_of(&back));
    assert!(
        said(&rig.tw, &events, pilot).contains("Salt Moth docks at Ceres Freeport."),
        "she docks"
    );
}

/// **Land and leave survives a restart** (acceptance 8): settled at Ahuna, the world reboots
/// from its folded store; the berth reads settled at the site; a day later the lift charges the
/// stay from the durable journey's arrival, and she docks at Ceres.
#[test]
fn sm2_land_and_leave_survives_a_restart() {
    let (mut rig, pilot, row) = aboard(0, 80, 120);
    let hull = ObjId(row.hull);
    let (_, t) = file_and_launch(&mut rig, pilot, hull, "+ship/undock ahuna");
    let since = arrival_of(&t);
    let _ = to_ic(&mut rig, since);
    let mut rig = reboot(rig);
    match berth_of(rig.tw.world(), hull) {
        Berth::Docked { def, .. } => assert_eq!(def.venue.as_str(), "ahuna"),
        other => panic!("not on the pad after the restart: {other:?}"),
    }
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(
        ship.contains("Salt Moth (Dray): settled on the pad at Ahuna Flank."),
        "{ship}"
    );
    let _ = to_ic(&mut rig, since + 86_400);
    let (_, back) = file_and_launch(&mut rig, pilot, hull, "+ship/lift ceres");
    assert_eq!((back.plan.landed_crew_days, back.plan.crew_days), (1, 2));
    let events = to_ic(&mut rig, arrival_of(&back));
    assert!(
        said(&rig.tw, &events, pilot).contains("Salt Moth docks at Ceres Freeport."),
        "she docks"
    );
}

/// **A departure for a site holds back the leg home** (decision 7): an empty tank plans the
/// shortfall and the leg home and refuses with `SITE_TANK_SHORT`, never a dark plan (reviewer
/// F1); a tank of exactly the outbound burn refuses with `SITE_TANK_SHORT`; a tonne more and a locker of exactly the
/// outbound crew-days refuses with `SITE_STORES_SHORT`; a crew-day more files; `SITE_HOME`
/// names the home route's landing floor.
#[test]
fn sm2_departure_for_a_site_holds_back_the_leg_home() {
    let (mut rig, pilot, row) = aboard(0, 0, 1);
    let vault = hold_vault(&rig.tw, &row);
    // An empty tank (reviewer F1): the passage out burns more than the tank holds, so the
    // reserve's home payload keeps no tank at all — never a dark plan. `+ship/plan` reads the
    // shortfall and the leg home; `+ship/undock` refuses with the site's tank sentence.
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    let lines: Vec<&str> = plan.lines().collect();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Burn 1 t (tank 0 t; 1 t short). ")),
        "{plan}"
    );
    let home = SITE_HOME
        .replace("{port}", "Ahuna Flank")
        .replace("{burn}", "1")
        .replace("{days}", "1")
        .replace("{throttle}", &floor_of("ahuna_ceres", 100).to_string());
    assert!(
        lines.contains(&home.as_str()),
        "{home}
{plan}"
    );
    assert!(!plan.contains("cannot be planned"), "{plan}");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert!(queued.is_empty());
    assert_eq!(
        out,
        SITE_TANK_SHORT
            .replace("{port}", "Ahuna Flank")
            .replace("{burn}", "2")
            .replace("{tank}", "0")
    );
    seed_pool(&mut rig, vault, "propellant", 1, LotId(9_200));
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert!(queued.is_empty());
    assert_eq!(
        out,
        SITE_TANK_SHORT
            .replace("{port}", "Ahuna Flank")
            .replace("{burn}", "2")
            .replace("{tank}", "1")
    );
    seed_pool(&mut rig, vault, "propellant", 1, LotId(9_201));
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert!(queued.is_empty());
    assert_eq!(
        out,
        SITE_STORES_SHORT
            .replace("{port}", "Ahuna Flank")
            .replace("{days}", "2")
            .replace("{have}", "1")
    );
    seed_pool(&mut rig, vault, "provisions", 1, LotId(9_202));
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    let payload: u64 = 2_000 + 8 + 100;
    let home = SITE_HOME
        .replace("{port}", "Ahuna Flank")
        .replace("{burn}", "1")
        .replace("{days}", "1")
        .replace(
            "{throttle}",
            &floor_of("ahuna_ceres", payload - 1_000 - 4).to_string(),
        );
    assert!(plan.lines().any(|l| l == home), "{home}\n{plan}");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert!(out.contains("Clearance filed for Ahuna Flank"), "{out}");
    assert_eq!(queued.len(), 1);
}

/// **The landing refusal's player sentence** (D-20-3; reviewer T8): at Ceres with a full tank
/// and an empty hold, Ahuna Flank at throttle 300 is under the landing law — thrust-to-weight
/// 703 permille, printed floored to hundredths as `0.70`, the body by its display name.
/// `+ship/plan` prints it without its last sentence; `+ship/undock` refuses with all of it and
/// burns nothing.
#[test]
fn sm2_the_landing_refusal_names_the_body_and_the_weight() {
    let (mut rig, pilot, _row) = aboard(0, 80, 120);
    let payload: u64 = 80_000 + 480 + 100;
    assert_eq!(
        tacsim::port::passage::landing_twr_permille(&dray(300), DRY_KG, payload, 28),
        Some(703)
    );
    let refused = LANDING_REFUSED
        .replace("{port}", "Ahuna Flank")
        .replace("{body}", "Ceres")
        .replace("{name}", "Salt Moth")
        .replace("{twr}", "0.70");
    assert_eq!(
        refused,
        "Ahuna Flank is on Ceres: at this load and throttle the Salt Moth lifts 0.70 times her \
         weight there, under the 1.20 a landing needs. Nothing was burned."
    );
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna throttle=300");
    let planned = refused.replace(" Nothing was burned.", "");
    assert!(plan.lines().any(|l| l == planned), "{plan}");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna throttle=300");
    assert_eq!(out, refused);
    assert!(queued.is_empty(), "nothing is burned");
}

/// **Rheasilvia under the moving sky** (decision 7; reviewers T4 and F3): at the lane's epoch a
/// full tank and an empty hold plan the passage out at 64 t and hold back the leg home at 37 t
/// and throttle 366 — `tests/landing.rs`'s `EPOCH_FIGURES`, here through `site_reserve` itself
/// — so the filing refuses 101 t against the tank's 80; at the passage's landing floor (457)
/// it still refuses at the epoch, and 180 IC days on it files. Settled on the pad, the tank
/// holds what the reserve kept back: the full-throttle lift overruns it, `+ship/plan ceres`'s
/// `Throttle:` line reads the tank aboard (nothing is sold on a pad) and names a throttle from
/// the leg home's landing floor up, and the lift at that throttle files.
#[test]
#[allow(clippy::too_many_lines)]
fn sm2_rheasilvia_holds_back_the_leg_home_under_the_moving_sky() {
    let mut rig = stocked_on(Roster::moving_sky());
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    load(&mut rig, &row, 0, 80, 120, 9_500);
    board(&mut rig, pilot);
    let home_line = |burn: u64, days: u32, throttle: u32| {
        SITE_HOME
            .replace("{port}", "Rheasilvia Commons")
            .replace("{burn}", &burn.to_string())
            .replace("{days}", &days.to_string())
            .replace("{throttle}", &throttle.to_string())
    };
    let plan = run(&mut rig.tw, pilot, "+ship/plan rheasilvia");
    let lines: Vec<&str> = plan.lines().collect();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Burn 64 t (tank 80 t; 16 t left). ")),
        "{plan}"
    );
    assert!(lines.contains(&home_line(37, 36, 366).as_str()), "{plan}");
    // The SM.2 fixer (the lead's item 2): a live refusal names the hint on its second line —
    // here, that no throttle from the floor fits today, and the first day of `+chart`'s year
    // on which the floor does: 123 IC days on, inside the 180 below
    // (`sm2_the_throttle_hint_counts_the_leg_home`).
    let later = THROTTLE_LATER_SITE
        .replace("{min}", "457")
        .replace("{pools}", "full")
        .replace("{date}", "0000-05-04")
        .replace("{offset}", "123");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock rheasilvia");
    assert!(queued.is_empty());
    assert_eq!(
        out,
        format!(
            "{}\n{later}",
            SITE_TANK_SHORT
                .replace("{port}", "Rheasilvia Commons")
                .replace("{burn}", "101")
                .replace("{tank}", "80")
        )
    );
    let (out, queued) = command(&mut rig, pilot, "+ship/undock rheasilvia throttle=457");
    assert!(queued.is_empty());
    assert_eq!(
        out,
        format!(
            "{}\n{later}",
            SITE_TANK_SHORT
                .replace("{port}", "Rheasilvia Commons")
                .replace("{burn}", "84")
                .replace("{tank}", "80")
        )
    );

    // In season: 180 IC days on (world rate 1000, so the charter's real clock barely moves).
    set_rate(&mut rig.tw, 1_000);
    let start = ic_now(&rig.tw);
    let _ = to_ic(&mut rig, start + 180 * 86_400);
    let plan = run(&mut rig.tw, pilot, "+ship/plan rheasilvia throttle=457");
    let lines: Vec<&str> = plan.lines().collect();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Burn 41 t (tank 80 t; 39 t left). ")),
        "{plan}"
    );
    assert!(lines.contains(&home_line(38, 34, 399).as_str()), "{plan}");
    let (_, t) = file_and_launch(
        &mut rig,
        pilot,
        hull,
        "+ship/undock rheasilvia throttle=457",
    );
    assert_eq!((t.plan.burn_t, t.plan.throttle_permille), (41, 457));
    assert_eq!(
        t.plan.legs.iter().map(|leg| leg.ascent_mps).sum::<u32>(),
        254
    );
    let arrival = voyage(&rig.tw, hull)
        .and_then(|v| v.arrival)
        .expect("the arrival row");
    assert_eq!(arrival.home_crew_days, 34);
    let _ = to_ic(&mut rig, arrival_of(&t));
    match berth_of(rig.tw.world(), hull) {
        Berth::Docked { def, .. } => assert_eq!(def.venue.as_str(), "rheasilvia"),
        other => panic!("not on the pad: {other:?}"),
    }

    // On the pad: 39 t aboard. The full-throttle lift overruns it; the hint names the throttle
    // that fits the tank as it stands, from the leg home's landing floor up.
    let vault = hold_vault(&rig.tw, &row);
    let tank = pool(&mut rig.tw, vault, "propellant");
    assert_eq!(tank, 39);
    let locker = pool(&mut rig.tw, vault, "provisions");
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    let lines: Vec<&str> = plan.lines().collect();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Burn ") && l.contains(" t short). ")),
        "{plan}"
    );
    let hint = lines
        .iter()
        .find(|l| l.starts_with("Throttle: "))
        .unwrap_or_else(|| panic!("a Throttle: line: {plan}"));
    let throttle: u32 = hint
        .strip_prefix("Throttle: with the tank and locker as they stand, throttle=")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|word| word.parse().ok())
        .unwrap_or_else(|| panic!("THROTTLE_FITS_PAD: {hint}"));
    let payload = tank * 1_000 + locker * 4 + 100;
    let floor = floor_of("rheasilvia_ceres", payload);
    assert!(
        (floor..1_000).contains(&throttle),
        "{throttle} from the floor {floor}: {hint}"
    );
    let burn: u64 = hint
        .split("burn ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|word| word.parse().ok())
        .unwrap_or_else(|| panic!("the hinted burn: {hint}"));
    assert!(burn <= tank, "{hint}");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert!(queued.is_empty());
    assert!(
        out.starts_with("The plan for Ceres Freeport burns "),
        "{out}"
    );
    let (out, queued) = command(
        &mut rig,
        pilot,
        &format!("+ship/lift ceres throttle={throttle}"),
    );
    assert!(out.contains("Clearance filed for Ceres Freeport"), "{out}");
    assert_eq!(queued.len(), 1);
}

/// The `Throttle:` line among `text`'s lines.
fn throttle_line(text: &str) -> &str {
    text.lines()
        .find(|l| l.starts_with("Throttle: "))
        .unwrap_or_else(|| panic!("a Throttle: line: {text}"))
}

/// The number after `prefix` in `line`, up to its first character that is not a digit.
fn number_after(line: &str, prefix: &str) -> i64 {
    line.split(prefix)
        .nth(1)
        .map(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("{prefix:?} in {line}"))
}

/// **The `Throttle:` line counts the leg home** (the SM.2 fixer, the lead's pre-commit item
/// 2): away from a pad the hint for a site destination asks what the filing asks — the
/// passage out and the leg home together in the tank. At the lane's epoch a full tank and an
/// empty hold fit Rheasilvia at no throttle from its landing floor (457) today
/// (`tests/landing.rs`'s year sweep: the season opens some four months on), so the hint
/// scans `+chart`'s window (the IC year, `WINDOW_DAYS`) and names the first later day the
/// floor fits — `+ship/plan` and the refused `+ship/undock` both say so, at the epoch itself;
/// the day before it, the hint names the same date one IC day on; on that day the
/// full-throttle departure still refuses, the hint names the highest throttle that fits, and
/// the departure at that throttle files. A queued depart reads the same refusal in
/// `+ship/plan`'s dry run (the lead's item 3).
#[test]
#[allow(clippy::too_many_lines)]
fn sm2_the_throttle_hint_counts_the_leg_home() {
    let mut rig = stocked_on(Roster::moving_sky());
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    load(&mut rig, &row, 0, 80, 120, 9_600);
    board(&mut rig, pilot);
    let head = THROTTLE_LATER_SITE
        .replace("{min}", "457")
        .replace("{pools}", "full");
    let head = head
        .split("{date}")
        .next()
        .expect("the line before its date")
        .to_owned();
    let epoch = ic_now(&rig.tw);
    let started = std::time::Instant::now();
    let plan = run(&mut rig.tw, pilot, "+ship/plan rheasilvia");
    println!(
        "+ship/plan rheasilvia at the epoch: {:?}",
        started.elapsed()
    );
    let later = throttle_line(&plan).to_owned();
    assert!(later.starts_with(&head), "{plan}");
    let offset = number_after(&later, "(in ");
    assert!(
        (1..=WINDOW_DAYS).contains(&offset),
        "inside +chart's window: {later}"
    );
    // The figures `sm2_rheasilvia_holds_back_the_leg_home_under_the_moving_sky` pins.
    assert_eq!(
        later,
        THROTTLE_LATER_SITE
            .replace("{min}", "457")
            .replace("{pools}", "full")
            .replace("{date}", "0000-05-04")
            .replace("{offset}", "123")
    );
    let started = std::time::Instant::now();
    let (out, queued) = command(&mut rig, pilot, "+ship/undock rheasilvia");
    println!(
        "+ship/undock rheasilvia at the epoch: {:?}",
        started.elapsed()
    );
    assert!(queued.is_empty());
    let short = SITE_TANK_SHORT
        .replace("{port}", "Rheasilvia Commons")
        .replace("{burn}", "101")
        .replace("{tank}", "80");
    assert_eq!(out, format!("{short}\n{later}"));
    // The queue's dry run reads the filing's refusal (without its last sentence), never the
    // hint: a waiting note keeps the bare sentence.
    let filed = run(&mut rig.tw, pilot, "+ship/order depart rheasilvia at=+2h");
    assert!(
        filed.contains("Order 1 filed: depart for Rheasilvia Commons"),
        "{filed}"
    );
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    let dry = plan
        .lines()
        .find(|l| l.starts_with("  Order 1, depart for Rheasilvia Commons: "))
        .unwrap_or_else(|| panic!("the dry run's depart: {plan}"));
    assert!(
        dry.ends_with(
            "; it would hold: The passage to Rheasilvia Commons and the leg home burn 101 t of \
             reaction mass; the tank holds 80 t."
        ),
        "{dry}"
    );
    let out = run(&mut rig.tw, pilot, "+ship/order clear");
    assert!(!out.is_empty());

    // The day before the named one (world rate 1000, as the moving-sky test): still no
    // throttle fits today, and the hint names the same date, one IC day on.
    set_rate(&mut rig.tw, 1_000);
    let day = epoch + offset * 86_400;
    let _ = to_ic(&mut rig, day - 86_400);
    let plan = run(&mut rig.tw, pilot, "+ship/plan rheasilvia");
    let eve = throttle_line(&plan).to_owned();
    assert_eq!(
        eve,
        later.replace(&format!("(in {offset} IC days)"), "(in 1 IC days)"),
        "{plan}"
    );
    let (out, queued) = command(&mut rig, pilot, "+ship/undock rheasilvia");
    assert!(queued.is_empty());
    assert_eq!(out.lines().nth(1), Some(eve.as_str()), "{out}");

    // On that day: full throttle still refuses, and the hint names a throttle that files.
    let _ = to_ic(&mut rig, day);
    let (out, queued) = command(&mut rig, pilot, "+ship/undock rheasilvia");
    assert!(queued.is_empty());
    assert!(
        out.starts_with("The passage to Rheasilvia Commons and the leg home burn "),
        "{out}"
    );
    let fits = out.lines().nth(1).expect("the hint").to_owned();
    assert!(
        fits.starts_with("Throttle: with the tank and locker full, throttle="),
        "{fits}"
    );
    let throttle = u32::try_from(number_after(&fits, "throttle=")).expect("a throttle");
    assert!((457..1_000).contains(&throttle), "{fits}");
    let burn = number_after(&fits, "burn ");
    let home = number_after(&fits, " t and ");
    assert!(burn + home <= 80, "{fits}");
    let plan = run(&mut rig.tw, pilot, "+ship/plan rheasilvia");
    assert_eq!(throttle_line(&plan), fits, "{plan}");
    let (_, t) = file_and_launch(
        &mut rig,
        pilot,
        hull,
        &format!("+ship/undock rheasilvia throttle={throttle}"),
    );
    assert_eq!(
        (
            i64::try_from(t.plan.burn_t).expect("t"),
            t.plan.throttle_permille
        ),
        (burn, throttle)
    );
}

/// **No day of `+chart`'s window fits** (SM2-FX-1): with 40 t of metal concentrate aboard, the
/// landing floor rises to 514 and the floor fits the passage to Rheasilvia and the leg home on
/// no day of the IC year, so `+ship/plan` and the refused `+ship/undock` name the whole window
/// — 365 IC days, the year `+chart <port>` scans — and no date.
#[test]
fn sm2_the_throttle_hint_names_no_day_when_the_year_has_none() {
    let mut rig = stocked_on(Roster::moving_sky());
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    load(&mut rig, &row, 40, 80, 120, 9_600);
    board(&mut rig, pilot);
    let none = THROTTLE_NONE_SITE
        .replace("{min}", "514")
        .replace("{pools}", "full")
        .replace("{window}", &WINDOW_DAYS.to_string());
    assert!(
        none.ends_with("today or in the next 365 IC days."),
        "{none}"
    );
    let started = std::time::Instant::now();
    let plan = run(&mut rig.tw, pilot, "+ship/plan rheasilvia");
    println!(
        "+ship/plan rheasilvia, a year with no day: {:?}",
        started.elapsed()
    );
    assert_eq!(throttle_line(&plan), none, "{plan}");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock rheasilvia");
    assert!(queued.is_empty());
    assert_eq!(out.lines().nth(1), Some(none.as_str()), "{out}");
}

/// **The queue's dry run reads the filing's checks** (the SM.2 fixer, the lead's pre-commit
/// item 3): with Ahuna's pad room gone, a depart queued for two hours on reads `SITE_CLOSED`
/// in `+ship/plan`'s sailing-plan block before it is due, the sentence the fire holds with;
/// Kerwan, open, reads its plan.
#[test]
fn sm2_the_queue_dry_run_reads_a_closed_site() {
    let mut rig = stocked();
    let pad = station_room(&rig.tw, "ahuna_port", "room.pad");
    rig.tw
        .world_mut()
        .set_flag(pad, Flags::GOING, true)
        .expect("flagged");
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    load(&mut rig, &row, 0, 80, 120, 9_700);
    board(&mut rig, pilot);
    let out = run(&mut rig.tw, pilot, "+ship/order depart ahuna at=+2h");
    assert!(
        out.contains("Order 1 filed: depart for Ahuna Flank"),
        "{out}"
    );
    let plan = run(&mut rig.tw, pilot, "+ship/plan kerwan");
    assert!(
        plan.lines().any(|l| l
            == "  Order 1, depart for Ahuna Flank: it would hold: Ahuna Flank is \
                           closed: its pad is not ready."),
        "{plan}"
    );
    let orders = run(&mut rig.tw, pilot, "+ship/orders");
    assert!(
        orders.contains("order 1, departure for Ahuna Flank"),
        "{orders}"
    );
    let out = run(&mut rig.tw, pilot, "+ship/order clear");
    assert!(!out.is_empty());
    let out = run(&mut rig.tw, pilot, "+ship/order depart kerwan at=+2h");
    assert!(out.contains(" filed: depart for Kerwan Floor"), "{out}");
    let plan = run(&mut rig.tw, pilot, "+ship/plan kerwan");
    let dry = plan
        .lines()
        .find(|l| l.starts_with("  Order ") && l.contains(", depart for Kerwan Floor: "))
        .unwrap_or_else(|| panic!("the dry run's depart: {plan}"));
    assert!(
        dry.contains(", depart for Kerwan Floor: payload ") && !dry.contains("hold"),
        "{dry}"
    );
}

/// **A closed site refuses departures and never closes the desk** (lead override 3): with
/// Ahuna's pad room gone, its Rooms line reads Missing and `+coop/enrol` at Ceres still works;
/// `+ship/plan ahuna` and `+ship/undock ahuna` answer `SITE_CLOSED`.
#[test]
fn sm2_a_closed_site_refuses_departures_and_never_closes_the_desk() {
    let mut rig = stocked();
    let pad = station_room(&rig.tw, "ahuna_port", "room.pad");
    rig.tw
        .world_mut()
        .set_flag(pad, Flags::GOING, true)
        .expect("flagged");
    let report = tacsim::port::readiness(rig.tw.world());
    assert!(
        report.lines.iter().any(|l| l.binding == Binding::Rooms
            && l.port.as_ref().map(CanonicalId::as_str) == Some("ahuna_port")
            && l.state == LineState::Missing),
        "the pad is missing"
    );
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    load(&mut rig, &row, 0, 80, 120, 9_300);
    board(&mut rig, pilot);
    let closed = SITE_CLOSED.replace("{site}", "Ahuna Flank");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert_eq!(out, closed);
    assert!(queued.is_empty());
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    let lines: Vec<&str> = plan.lines().collect();
    assert_eq!(
        lines.get(..2),
        Some(&["Plan for Ahuna Flank:", closed.as_str()][..]),
        "{plan}"
    );
    // Kerwan is open.
    let (out, _) = command(&mut rig, pilot, "+ship/undock kerwan");
    assert!(out.contains("Clearance filed for Kerwan Floor"), "{out}");
}

/// **An unreadable site map closes the pad, never the desk** (lead override 3; reviewer F2):
/// Ahuna's port map document rots on a kept world; after the restart the readiness report's
/// Map line for `ahuna_port` reads Missing, the port is still named a site, `+coop/enrol` at
/// Ceres works, and a departure for Ahuna — live, planned or queued — answers `SITE_CLOSED`.
#[test]
fn sm2_an_unreadable_site_map_closes_the_pad_and_never_the_desk() {
    let mut rig = stocked();
    let ops = rig.tw.take_module_ops();
    fold_module_ops_into(&mut rig.store, ops);
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ahuna_port")).to_string();
    let slot = rig
        .store
        .get_mut(tacsim::NS)
        .and_then(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == key))
        .expect("Ahuna's map document");
    slot.1 = serde_json::json!({ "schema": "x" });
    let mut rig = reboot(rig);
    assert!(matches!(
        store(&rig.tw).port_map(&lane_id(), &cid("ahuna_port")),
        PortMapState::Unreadable
    ));
    let report = tacsim::port::readiness(rig.tw.world());
    assert!(
        report.lines.iter().any(|l| l.binding == Binding::Map
            && l.port.as_ref().map(CanonicalId::as_str) == Some("ahuna_port")
            && l.state == LineState::Missing),
        "the map is missing"
    );
    assert!(report.sites.contains(&cid("ahuna_port")), "still a site");
    assert!(
        tacsim::port::enrol_gate(&report).is_ok(),
        "{:?}",
        tacsim::port::enrol_gate(&report).err()
    );
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    load(&mut rig, &row, 0, 80, 120, 9_400);
    board(&mut rig, pilot);
    let closed = SITE_CLOSED.replace("{site}", "Ahuna Flank");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert_eq!(out, closed);
    assert!(queued.is_empty());
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    assert_eq!(plan.lines().nth(1), Some(closed.as_str()), "{plan}");
    // The queue's depart holds on the same refusal, burning nothing.
    let out = run(&mut rig.tw, pilot, "+ship/order depart ahuna");
    assert!(
        out.contains("Order 1 filed: depart for Ahuna Flank"),
        "{out}"
    );
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        assert!(
            queued
                .iter()
                .all(|s| s.action.key().owner.as_str() != tacsim::NS),
            "nothing is burned"
        );
        land_all(&mut rig, &queued);
    }
    let waits = WAITING_LINE
        .replace("{n}", "1")
        .replace("{reason}", &closed);
    let log: Vec<String> = match store(&rig.tw).orders_state(hull) {
        tacsim::port::orders::OrdersState::Present(orders) => {
            orders.log.iter().map(|l| l.text.clone()).collect()
        }
        other => panic!("the sailing plan: {other:?}"),
    };
    assert_eq!(log, vec![waits]);
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
}

/// **`+ship/lift` is `+ship/undock`** (the brief's item 11): at Ceres, on two worlds built the
/// same way, `+ship/lift stickney` and `+ship/undock stickney` file the same sailing plan.
#[test]
fn sm2_lift_is_undock() {
    let filed = |line: &str| -> Voyage {
        let (mut rig, pilot, row) = aboard(0, 80, 120);
        let (out, queued) = command(&mut rig, pilot, line);
        assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
        assert_eq!(queued.len(), 1);
        voyage(&rig.tw, ObjId(row.hull)).expect("filed")
    };
    let lifted = filed("+ship/lift stickney");
    let undocked = filed("+ship/undock stickney");
    assert_eq!(lifted, undocked);
}

// ---------------------------------------------------------------------------
// 7. +chart
// ---------------------------------------------------------------------------

/// Ceres's site destinations in venue order: (label, tenure parts, on Ceres).
const CHART_SITES: [(&str, &str, bool); 6] = [
    ("Ahuna Flank", "16 open", true),
    ("Freeport Commons", "16 commons", true),
    ("Juling Shadow", "16 open", true),
    ("Kerwan Floor", "16 open", true),
    (
        "Occator Rim",
        "12 open; 4 held by Occator Water & Propellant",
        true,
    ),
    ("Rheasilvia Commons", "16 commons", false),
];

/// The line after `+chart`'s passage line for `port`, and the one after that.
fn under<'a>(lines: &[&'a str], port: &str) -> (&'a str, &'a str, &'a str) {
    let at = lines
        .iter()
        .position(|l| l.starts_with(&format!("  {port}: ")))
        .unwrap_or_else(|| panic!("no line for {port}"));
    let after = |n: usize| lines.get(at + n).copied().unwrap_or("");
    (lines[at], after(1), after(2))
}

/// **`+chart` lists the sites with their survey words** (§1 beat 2; the brief's item 12):
/// keyless, every site reads "unavailable" and its exact tenure, and no Ceres site carries a
/// direct chord; a pilot settled on Ahuna's pad charts the Freeport without one either; keyed,
/// each survey reads the words of the site's province, computed here from `province_at`.
#[test]
fn sm2_chart_lists_the_sites_with_their_survey_words() {
    let (mut rig, pilot, row) = aboard(0, 80, 120);
    let chart = run(&mut rig.tw, GOD, "+chart");
    let lines: Vec<&str> = chart.lines().collect();
    for (label, tenure, on_ceres) in CHART_SITES {
        let (line, survey, tenure_line) = under(&lines, label);
        assert_eq!(survey, SURVEY_UNAVAILABLE, "{chart}");
        assert_eq!(tenure_line, TENURE_LINE.replace("{parts}", tenure));
        assert_eq!(!line.contains("the direct chord"), on_ceres, "{line}");
    }
    let (stickney, next, _) = under(&lines, "Stickney Yards");
    assert!(stickney.contains("the direct chord"), "{stickney}");
    assert!(!next.starts_with("    "), "a station has no survey");
    // Settled on Ahuna's pad: the Freeport is on the same body.
    let (_, t) = file_and_launch(&mut rig, pilot, ObjId(row.hull), "+ship/undock ahuna");
    let _ = to_ic(&mut rig, arrival_of(&t));
    let chart = run(&mut rig.tw, pilot, "+chart");
    let lines: Vec<&str> = chart.lines().collect();
    let (freeport, _, _) = under(&lines, "Ceres Freeport");
    assert!(!freeport.contains("the direct chord"), "{freeport}");

    // Keyed: the words are the province's.
    let mut tw = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), HashMap::new(), true);
    let chart = run(&mut tw, GOD, "+chart");
    let lines: Vec<&str> = chart.lines().collect();
    let pack = lane();
    let k = mac(KEY_1);
    for site in &SITES {
        let ground = body_ground(&pack, site.body);
        let province = site_province(&place(&pack, site.loc), &ground).expect("a province");
        let words = read_r0(&province_at(&k, &ground, province).expect("R0"));
        let text: Vec<String> = OreFamily::ALL
            .iter()
            .zip(words)
            .map(|(family, word)| format!("{} {}", family.label(), word.name()))
            .collect();
        let (_, survey, _) = under(&lines, site.label);
        assert_eq!(
            survey,
            SURVEY_LINE.replace("{words}", &text.join(", ")),
            "{}",
            site.label
        );
    }
}

// ---------------------------------------------------------------------------
// 8. A kept world takes v9
// ---------------------------------------------------------------------------

/// **A world on lane v8 takes v9 by the shelf** (`commerce.rs`'s A.8 model): booted on the v8
/// shape, the world has the twelve site atlas packs latched at 1 — they ride the boot list
/// whatever the lane version — and no site port map; rebooted into the shipped roster it plans
/// and applies twenty-four creates and ten updates (the eight bodies, the Freeport's place and
/// Ceres's port) over twenty-six unchanged entries, and every site is laid up with Ceres's and
/// Vesta's tables installed.
#[test]
fn sm2_a_world_on_lane_v8_takes_v9_by_the_shelf() {
    let v8 = lane_variant(8, strip_sites);
    assert_eq!(
        parse_pack(&v8).expect("v8").manifest().digest().to_hex(),
        LANE_V8_DIGEST
    );
    let mut rig = boot(Roster::seeded(v8));
    let shelf = run(&mut rig.tw, GOD, "@atlas/pack list");
    for (name, _) in SITE_SOURCES {
        assert!(shelf.contains(&format!(" {name}@1 (")), "{name}:\n{shelf}");
    }
    for entry in SITE_PORTS {
        assert!(
            matches!(
                store(&rig.tw).port_map(&lane_id(), &cid(entry)),
                PortMapState::Absent
            ),
            "{entry} before the upgrade"
        );
    }
    let Rig {
        mut tw,
        oracle,
        typed,
        ..
    } = rig;
    let folded = mush_testkit::fold_module_ops(tw.take_module_ops());
    let tw = tw
        .reboot_into(Roster::bare().modules(), folded.clone())
        .expect("the reboot boots");
    let mut rig = Rig {
        tw,
        oracle,
        roster: Roster::bare(),
        store: folded,
        typed,
    };
    with_offer_keys(&mut rig.tw);
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        plan.contains(&format!(
            "Plan for {LANE_PACK}@{LANE_VERSION}: 24 create, 10 update, 0 retire, 26 unchanged"
        )),
        "{plan}"
    );
    assert!(
        plan.lines()
            .any(|l| l.contains("update") && l.contains("ceres_port (port)")),
        "{plan}"
    );
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{LANE_VERSION}: 24 created, 10 updated, 0 retired, 26 unchanged."
        )),
        "{plan}\n{apply}"
    );
    for entry in SITE_PORTS {
        let map = port_map(&rig.tw, entry);
        let row = map.site.expect("laid up");
        assert_eq!(row.blocks.len(), 16, "{entry}");
        assert!(row.clerk.is_some(), "{entry}");
    }
    assert_eq!(
        installed_bodies(rig.tw.world()),
        vec![cid("ceres"), cid("vesta")]
    );
    no_refused_writes(&rig.tw);
}

/// **The apply installs the lane's tables** (reviewer T2; the `PortApplier`'s part (g)): a
/// world seeded on lane v8 alone boots with no body table (v8 authors none, and its store
/// loads no other version for the boot pass to read); applying v9 off the shelf lays the sites
/// up and installs Ceres's and Vesta's tables before any reboot — from the applied pack
/// itself, since the store does not load v9 (part (g)'s decode, the builder's decision 3).
#[test]
fn sm2_the_apply_installs_the_lane_tables() {
    let roster = Roster {
        seed_lane: Some(lane_variant(8, strip_sites)),
        shelf: Some(vec![
            mush_content::load_pack(LANE_JSON).expect("the shipped lane verifies"),
        ]),
        ..Roster::default()
    };
    let mut rig = boot(roster);
    assert_eq!(
        store(&rig.tw).applied_in(rig.tw.world(), &lane_id()),
        Some(8)
    );
    assert!(
        installed_bodies(rig.tw.world()).is_empty(),
        "lane v8 authors no table"
    );
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{LANE_VERSION}: 24 created, 10 updated, 0 retired, 26 unchanged."
        )),
        "{plan}\n{apply}"
    );
    assert_eq!(
        installed_bodies(rig.tw.world()),
        vec![cid("ceres"), cid("vesta")],
        "part (g) installs them at the apply"
    );
}
