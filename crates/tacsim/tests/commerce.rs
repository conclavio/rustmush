//! **Doc 18 cut IC8 acceptance — finite stock and port commerce** (doc 18 §4.3
//! "Finite seeded stock", "Presence", "The capacity function", "Vendors and buyers",
//! "Stock and capital", §4.7 parts d–e; D-18-12, D-18-26, D-18-32, D-18-39; Q-18-31;
//! lead rulings L1–L25 of the IC8 brief).
//!
//! Every unit of trade matter at the lane's two ports is minted once by a per-port
//! stock clerk acting on the `tacsim.port` tick, receipted, dissolved into a
//! storefront pool and listed as a Draw; a chartered pilot at the Exchange quotes and
//! accepts with no destination and the Draw loads the berthed hold, under the port's
//! presence gate, the hull's capacity provider and the port institutions' party caps;
//! the boards count down to SOLD OUT; the standing bids are posted as documents; and a
//! player's lapsed lot listing is released by the tick acting as the vault's owner.
//!
//! Composed on the composition root (`server_modules_with_tacsim` and the wired
//! test-world layer, no sheet, so a spawned player types and walks: `coop.rs`'s
//! `Roster::bare`). Every boot asserts no namespace write was refused; every drain
//! asserts no kept tacsim submission is GOD's.

mod support;

#[path = "support/vertex.rs"]
mod vertex;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, LIMBO, ModuleWrite, ObjId, ObjKind,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    ContractState, EconomyConfig, EconomyStore, EscalationKind, PartyCap, PartyCapSource, VaultId,
};
use mush_inventory::{EndpointRef, InventoryHandles, Lineage, LotId};
use mush_market::{CapacityProvider, ListingId, MarketStore, Presence, VenueGate};
use mush_org::{OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops, fold_module_ops_into};
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::{
    HOLD_ELSEWHERE, HOLD_NOT_HERE, HOLD_UNDER_WAY, HullRow, NOT_IN_PORT, NOT_YOUR_CHARTER,
    PortCapacity, PortDef, PortHandles, PortMap, PortMapState, PortPartyCaps, PortVenueGate,
    StockIssueId, room_key,
};
use tacsim::{TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};

/// **The published lane v2's digest** (doc 18 cut IC3), copied from `tests/lane.rs`
/// before cut IC8 re-signed the lane at v3: test 16 rebuilds those bytes.
const LANE_V2_DIGEST: &str = "3f7e1a962eedf4bf4fcdd06c83ba893c555c11579dd1b35a20901bef596354e0";

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

/// How a test roster is composed: `coop.rs`'s `Roster::bare` (no sheet, the test
/// world's layer), with an optional lane booted in place of the shipped one (an
/// operator seed, `ports.rs`'s v1 pattern) and an optional shelf.
#[derive(Clone, Default)]
struct Roster {
    /// A lane booted instead of the shipped one, as an operator seed.
    seed_lane: Option<String>,
    /// A shelf replacing the atlas module's compiled one.
    shelf: Option<Vec<mush_content::Pack>>,
    /// An extra lane version the layer loads (an operator seed beside the shipped
    /// lane), so an update to it is read by the report and the arm.
    extra_lane: Option<String>,
}

impl Roster {
    fn bare() -> Roster {
        Roster::default()
    }

    fn seeded(lane: String) -> Roster {
        Roster {
            seed_lane: Some(lane),
            shelf: None,
            extra_lane: None,
        }
    }

    fn with_shelf(mut self, shelf: Vec<mush_content::Pack>) -> Roster {
        self.shelf = Some(shelf);
        self
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
            mods[at] = Box::new(
                mush_atlas::AtlasModule::with_config(mush_atlas::AtlasConfig::default())
                    .with_shelf(with_stations(shelf)),
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
            None => {
                mods.push(
                    comp::content_module_with(true, true).expect("the content module composes"),
                );
            }
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

/// A booted world, its oracle and the cumulative store (`coop.rs`'s rig).
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
            mush_core::OfferSecret::new(1, [0x41; 32]).expect("an offer secret"),
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

/// Boot, settle and stock both ports.
fn stocked() -> Rig {
    let mut rig = boot(Roster::bare());
    stock(&mut rig);
    rig
}

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

fn market(tw: &TestWorld) -> &MarketStore {
    tw.world().module_state::<MarketStore>().expect("market")
}

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&lane_id(), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn port_def(json: &str, entry: &str) -> PortDef {
    let pack = load_pack(json).expect("the lane verifies");
    match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        _ => panic!("{entry} is a port entry"),
    }
}

fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = port_def(LANE_JSON, entry);
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn c07(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c07_exchange")
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let _ = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(location(tw, who), Some(room), "the teleport landed");
}

/// The clerk Thing of one port entry.
fn clerk(tw: &TestWorld, entry: &str) -> ObjId {
    ObjId(port_map(tw, entry).clerk.expect("a clerk").thing)
}

/// A vendor's storefront vault.
fn storefront(tw: &TestWorld, entry: &str, org: &str) -> VaultId {
    VaultId(port_map(tw, entry).vendors[&cid(org)].vault)
}

/// The live listing on one storefront pool.
fn listing_on(tw: &TestWorld, entry: &str, org: &str, good: &str) -> Option<ListingId> {
    market(tw).lot_listing_of(EndpointRef(storefront(tw, entry, org)), &cid(good))
}

/// A fresh request/commit token per landing, across the process.
fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0xa8_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Drive one queued submission the way the world task does (`port_tick.rs`'s
/// `land_one`), keeping its typed post-images for a reboot.
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

/// Every queued submission of the port's lanes (tacsim's mint, the clerk's dissolve,
/// the market's legs), dropping other modules'; asserts none kept of tacsim's is GOD's.
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

/// One port-tick fire (6 s on), its submissions drained.
fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// Land every submission, each of which must land.
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

fn status(tw: &mut TestWorld) -> String {
    run(tw, GOD, "@tacsim/port status")
}

/// Whether every Pools and Listings line of `entries` (and the market facts line)
/// reads `[ready]`.
fn stock_ready(tw: &mut TestWorld) -> (bool, String) {
    let out = status(tw);
    let gated: Vec<&str> = out
        .lines()
        .filter(|line| {
            line.contains(" vendor ") || line.contains(" ask ") || line.starts_with("market facts")
        })
        .collect();
    let ready = gated.len() > 1 && gated.iter().all(|line| line.ends_with("[ready]"));
    (ready, out)
}

/// **Fire and land the port tick until both ports are stocked and listed.** Bounded.
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        if stock_ready(&mut rig.tw).0 {
            return;
        }
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
    }
    let (_, out) = stock_ready(&mut rig.tw);
    panic!("the ports were not stocked within 40 fires:\n{out}");
}

/// Read one typed value through an owner-stamped transaction.
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

fn inventory(tw: &TestWorld) -> mush_inventory::InventoryFamilies {
    tw.world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone()
}

fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = inventory(tw);
    let key = mush_inventory::pool_key(EndpointRef(vault), &cid(good)).expect("a pool key");
    typed(tw, &families.pools, &key).map_or(0, |scalar| {
        u64::try_from(scalar.current()).expect("a non-negative pool")
    })
}

fn receipts(tw: &mut TestWorld) -> Vec<(StockIssueId, tacsim::port::StockReceipt)> {
    let handles = tw
        .world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles");
    let mut out = Vec::new();
    for entry in ["ceres_port", "stickney_port"] {
        let map = port_map(tw, entry);
        for row in map.vendors.values() {
            for stock in &row.stock {
                let id = StockIssueId::parse(&stock.id).expect("an issue id");
                if let Some(receipt) = typed(tw, &handles.families.port_stock, &id) {
                    out.push((id, receipt));
                }
            }
        }
    }
    out
}

fn account(name: &str, seed: i64) -> AccountId {
    AccountId::mint(name, seed).expect("an account id")
}

/// A pilot on a gateway account of their own, standing at the desk.
fn pilot(rig: &mut Rig, who: &str, seed: i64) -> ObjId {
    let pilot = rig.tw.spawn_player(who);
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(pilot, account(&format!("{who}Acct"), seed));
    }
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    pilot
}

/// An enrolled pilot (`Salt Moth`), standing at C07.
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
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, pilot, exchange);
    (pilot, row)
}

/// The hold vault of a chartered hull.
fn hold_vault(tw: &TestWorld, row: &HullRow) -> VaultId {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    economy(tw)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id
}

/// A principal's own `tacsim.credit` account and its posted and held balances.
fn balances(tw: &TestWorld, owner: &PrincipalId) -> (i64, i64) {
    let instrument = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit");
    economy(tw)
        .snapshot()
        .accounts_of(owner)
        .find(|a| a.instrument == instrument)
        .map_or((0, 0), |a| {
            (a.posted_balance.minor(), a.held_amount.minor())
        })
}

fn treasury_account(tw: &TestWorld) -> u64 {
    port_map(tw, "ceres_port").treasury.expect("a treasury")
}

/// Mint `minor` into `who`'s own credit account from the treasury.
fn top_up(tw: &mut TestWorld, who: ObjId, minor: i64) {
    let instrument = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit");
    let account = economy(tw)
        .snapshot()
        .accounts_of(&PrincipalId::Object(who))
        .find(|a| a.instrument == instrument)
        .map(|a| a.id)
        .expect("the pilot's account");
    let treasury = treasury_account(tw);
    let out = run(tw, GOD, &format!("@econ/mint {treasury}={account} {minor}"));
    assert!(!out.contains("Usage"), "{out}");
}

/// The newest quote `buyer` holds.
fn newest_quote(tw: &TestWorld, buyer: ObjId) -> mush_market::QuoteId {
    market(tw)
        .quotes_of_buyer(&PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
        .expect("a quote")
}

fn refused_settlements(tw: &TestWorld) -> usize {
    economy(tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .count()
}

/// Quote `qty` of `listing` and accept it with no destination; the accept's reply
/// and the quote.
fn buy(
    rig: &mut Rig,
    pilot: ObjId,
    listing: ListingId,
    qty: u64,
) -> (String, mush_market::QuoteId) {
    let quoted = run(
        &mut rig.tw,
        pilot,
        &format!("+market/quote {listing}={qty}"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, pilot);
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    (accepted, quote)
}

/// Buy and land the Draw.
fn buy_and_land(rig: &mut Rig, pilot: ObjId, listing: ListingId, qty: u64) -> mush_market::QuoteId {
    let (accepted, quote) = buy(rig, pilot, listing, qty);
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{accepted}");
    assert_eq!(queued[0].actor, pilot, "the Draw lands as the pilot");
    land_all(rig, &queued);
    quote
}

/// `look` in `room` as `who`, after a teleport.
fn look_in(tw: &mut TestWorld, who: ObjId, room: ObjId) -> String {
    teleport(tw, who, room);
    run(tw, who, "look")
}

/// The board block of a `look`: the heading and the lines after it.
fn board(out: &str) -> Vec<String> {
    out.lines()
        .skip_while(|line| *line != tacsim::port::board::BOARD_HEADING)
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(str::to_owned)
        .collect()
}

/// The lane's bytes as JSON, edited by `edit`, at `version`, re-signed.
fn lane_variant(version: u32, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = serde_json::json!(version);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
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
// 1. The applier submits nothing
// ---------------------------------------------------------------------------

/// **Lens 8: the applier submits nothing.** Right after the boot (the content pass)
/// the engine holds no queued authoritative submission; the first one appears on the
/// first `tacsim.port` fire, and its actor is the Ceres stock clerk, a Thing.
#[test]
fn ic8_the_applier_submits_nothing() {
    let mut rig = boot_unsettled(Roster::bare());
    assert_eq!(
        rig.tw.engine().queued_authoritative_actors().count(),
        0,
        "the content pass queued an authoritative submission"
    );
    let ceres = clerk(&rig.tw, "ceres_port");
    let stickney = clerk(&rig.tw, "stickney_port");
    for thing in [ceres, stickney] {
        let obj = rig.tw.world().get(thing).expect("the clerk");
        assert_eq!(obj.kind, ObjKind::Thing);
        assert!(
            obj.flags.contains(mush_core::Flags::DARK),
            "the clerk is DARK"
        );
    }
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1000));
    let first = rig
        .tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the first fire submits");
    assert_eq!(
        first.actor, ceres,
        "the first submission is the Ceres stock clerk's"
    );
    assert_eq!(first.action.key().id.as_str(), tacsim::port::STOCK_ACTION);
    let rest = drain(&mut rig.tw);
    assert!(
        rest.iter().all(|s| s.actor == stickney),
        "the other is the Stickney clerk's: {rest:?}"
    );
}

// ---------------------------------------------------------------------------
// 2–3. Every pool seeded exactly once
// ---------------------------------------------------------------------------

/// **Lens 1: the ticks seed every pool exactly once.** Each of the eight stock rows
/// ends with one receipt, one `Lineage::Origin` lot (dissolved into the storefront
/// pool) and a pool equal to the receipt's quantity; every ask is listed at its pool;
/// ten further fires submit nothing.
#[test]
fn ic8_the_ticks_seed_every_pool_exactly_once() {
    let mut rig = boot(Roster::bare());
    // The first fire mints; the lot stands with its origin before the dissolve.
    let (_, queued) = fire(&mut rig);
    assert_eq!(queued.len(), 2, "one mint per clerk");
    land_all(&mut rig, &queued);
    let families = inventory(&rig.tw);
    let minted = receipts(&mut rig.tw);
    assert_eq!(minted.len(), 2);
    for (_, receipt) in &minted {
        let lot = typed(&mut rig.tw, &families.lots, &LotId(receipt.lot)).expect("the lot");
        match lot.lineage() {
            Lineage::Origin { source } => {
                assert_eq!(source.provider.as_str(), "tacsim");
                assert_eq!(source.kind.as_str(), "port.stock");
                assert_eq!(
                    source.id.as_str(),
                    format!(
                        "{}/{}/{}/{}",
                        receipt.pack, receipt.entry, receipt.org, receipt.good
                    )
                );
                assert_eq!(source.revision, Some(u64::from(receipt.issue)));
            }
            other => panic!("not an origin lot: {other:?}"),
        }
    }
    // The readiness texts on the way (fixer T13): a minted, undissolved row is not
    // seeded yet; once every row is seeded, an ask not yet listed waits to be listed.
    let ceres = port_map(&rig.tw, "ceres_port");
    let exchange = VaultId(ceres.vendors[&cid("freeport_exchange")].vault);
    let report = status(&mut rig.tw);
    assert!(
        report.contains(&format!(
            "  ceres_port vendor freeport_exchange: Freeport Exchange, storefront vault #{}; 0 \
             of 1 stock rows seeded. [pending]",
            exchange.0
        )),
        "{report}"
    );
    assert!(
        report.contains("  ceres_port ask freeport_exchange metals: not seeded yet. [pending]"),
        "{report}"
    );
    let waiting = "  ceres_port ask freeport_exchange metals: 4000 t in the storefront wait to be \
                   listed. [pending]";
    let mut saw_waiting = false;
    for _ in 0..40 {
        let (ready, report) = stock_ready(&mut rig.tw);
        saw_waiting |= report.contains(waiting);
        if ready {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert!(saw_waiting, "the seeded, unlisted ask waited to be listed");
    stock(&mut rig);
    let report = status(&mut rig.tw);
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    let alloys = ceres.vendors[&cid("freeport_exchange")].bids[&cid("structural_alloys")].listing;
    let occator = VaultId(ceres.vendors[&cid("occator")].vault);
    for line in [
        format!(
            "  ceres_port vendor freeport_exchange: Freeport Exchange, storefront vault #{}; 1 of \
             1 stock rows seeded. [ready]",
            exchange.0
        ),
        format!(
            "  ceres_port vendor occator: Occator Water & Propellant, storefront vault #{}; 2 of \
             2 stock rows seeded. [ready]",
            occator.0
        ),
        format!(
            "  ceres_port ask freeport_exchange metals: listing #{metals} at 200.00cr/t, 4000 t \
             listed; the storefront holds 4000 t. [ready]"
        ),
        format!(
            "  ceres_port bid freeport_exchange structural_alloys: listing #{alloys} at \
             520.00cr/t, 7000 of 7000 t wanted. [ready]"
        ),
    ] {
        assert!(report.contains(&line), "want `{line}`:\n{report}");
    }
    let all = receipts(&mut rig.tw);
    assert_eq!(all.len(), 8, "Ceres 4 and Stickney 4");
    for (id, receipt) in &all {
        assert_eq!(receipt.issue_id().expect("derives"), *id);
        assert_eq!(
            pool(&mut rig.tw, receipt.vault, receipt.good.as_str()),
            receipt.qty
        );
        assert!(
            typed(&mut rig.tw, &families.lots, &LotId(receipt.lot)).is_none(),
            "the lot is dissolved"
        );
    }
    for entry in ["ceres_port", "stickney_port"] {
        let def = port_def(LANE_JSON, entry);
        for vendor in &def.vendors {
            for ask in &vendor.asks {
                let id = listing_on(&rig.tw, entry, vendor.org.as_str(), ask.good.as_str())
                    .unwrap_or_else(|| panic!("{entry} {} is listed", ask.good));
                let listed = market(&rig.tw).listing(id).expect("the listing").quantity;
                let vault = storefront(&rig.tw, entry, vendor.org.as_str());
                assert_eq!(listed, pool(&mut rig.tw, vault, ask.good.as_str()));
            }
        }
    }
    for _ in 0..10 {
        let (_, queued) = fire(&mut rig);
        assert!(queued.is_empty(), "a further fire submitted {queued:?}");
    }
    no_refused_writes(&rig.tw);
}

/// **Lens 1's kill and reboot**: after the first mints land and before seeding
/// finishes, a reboot (the module documents folded, the typed post-images replayed
/// through `hydrate_action_state`) re-mints nothing — every landing after it lands,
/// the receipts are exactly the eight rows, and each pool is its receipts' sum.
#[test]
fn ic8_a_reboot_mid_seed_re_mints_nothing() {
    let mut rig = boot(Roster::bare());
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(receipts(&mut rig.tw).len(), 2);
    let mut rig = reboot(rig);
    assert_eq!(receipts(&mut rig.tw).len(), 2, "the receipts survive");
    stock(&mut rig);
    let all = receipts(&mut rig.tw);
    assert_eq!(all.len(), 8);
    let mut sums: BTreeMap<(VaultId, String), u64> = BTreeMap::new();
    for (_, receipt) in &all {
        *sums
            .entry((receipt.vault, receipt.good.to_string()))
            .or_insert(0) += receipt.qty;
    }
    for ((vault, good), sum) in sums {
        assert_eq!(pool(&mut rig.tw, vault, &good), sum, "{good}");
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 4. The clerk's authority
// ---------------------------------------------------------------------------

/// **The clerk may take from every storefront of its port, and nothing else**: its
/// direct `vault.withdraw` grants (lead ruling L25), never the other port's.
#[test]
fn ic8_the_clerk_may_take_from_every_storefront_and_nothing_else() {
    let mut rig = stocked();
    let (_, row) = chartered(&mut rig, "Pilot", 1);
    let hold = hold_vault(&rig.tw, &row);
    let w = rig.tw.world();
    for (entry, other) in [
        ("ceres_port", "stickney_port"),
        ("stickney_port", "ceres_port"),
    ] {
        let clerk = clerk(&rig.tw, entry);
        for row in port_map(&rig.tw, entry).vendors.values() {
            assert!(
                mush_inventory::may_take(w, clerk, EndpointRef(VaultId(row.vault))),
                "{entry}'s clerk takes from its storefront #{}",
                row.vault
            );
        }
        for row in port_map(&rig.tw, other).vendors.values() {
            assert!(
                !mush_inventory::may_take(w, clerk, EndpointRef(VaultId(row.vault))),
                "{entry}'s clerk takes nothing of {other}'s"
            );
        }
        assert!(
            !mush_inventory::may_take(w, clerk, EndpointRef(hold)),
            "nor a pilot's hold"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. The buy path
// ---------------------------------------------------------------------------

/// **Lens 2: a no-argument list, a quote and a no-destination accept load the hold.**
/// The Draw lands as the pilot, the hold's metals pool rises by the quantity, the
/// listing falls by it, the money moves once, the contract settles on the sweeps,
/// and no `SettlementRefused` page appears.
#[test]
fn ic8_a_no_argument_list_quote_accept_loads_the_hold() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hold = hold_vault(&rig.tw, &row);
    let list = run(&mut rig.tw, pilot, "+market/list");
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    assert!(
        list.contains(&format!(
            "#{metals} metal concentrate — 4000 × 200.00cr [ceres]"
        )),
        "the Ceres board: {list}"
    );
    // Only the room's venue: the market's no-venue page would carry Stickney's rows
    // too (fixer T9).
    assert!(!list.contains("[stickney]"), "only Ceres listings: {list}");
    let before = market(&rig.tw).listing(metals).expect("listed").quantity;
    let (posted_before, _) = balances(&rig.tw, &PrincipalId::Object(pilot));
    let quote = buy_and_land(&mut rig, pilot, metals, 10);
    assert_eq!(
        pool(&mut rig.tw, hold, "metals"),
        10,
        "the hold's pool rose by ten"
    );
    assert_eq!(
        market(&rig.tw).listing(metals).expect("listed").quantity,
        before - 10,
        "the listing fell by ten"
    );
    let contract = market(&rig.tw)
        .quote(quote)
        .and_then(|q| q.contract)
        .expect("a contract");
    for _ in 0..6 {
        sweep(&mut rig.tw, 61);
        let _ = drain(&mut rig.tw);
        if matches!(
            economy(&rig.tw)
                .snapshot()
                .contract(contract)
                .map(|c| c.state),
            Some(ContractState::Settled { .. })
        ) {
            break;
        }
    }
    assert!(
        matches!(
            economy(&rig.tw)
                .snapshot()
                .contract(contract)
                .map(|c| c.state),
            Some(ContractState::Settled { .. })
        ),
        "the contract settles on the sweeps"
    );
    let (posted_after, held_after) = balances(&rig.tw, &PrincipalId::Object(pilot));
    assert_eq!(
        posted_before - posted_after,
        10 * 20_000,
        "the money moved once"
    );
    assert_eq!(held_after, 0, "nothing stays held");
    assert_eq!(refused_settlements(&rig.tw), 0);
}

// ---------------------------------------------------------------------------
// 6. Capacity refusals
// ---------------------------------------------------------------------------

/// Quote and accept, expecting a refusal with exactly `line` and nothing charged.
fn refused_with(rig: &mut Rig, pilot: ObjId, listing: ListingId, qty: u64, line: &str) {
    let credit_before = run(&mut rig.tw, pilot, "+credit");
    let balances_before = balances(&rig.tw, &PrincipalId::Object(pilot));
    let (accepted, quote) = buy(rig, pilot, listing, qty);
    assert!(accepted.contains(line), "want `{line}`:\n{accepted}");
    assert!(
        market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.contract.is_none()),
        "no contract"
    );
    assert_eq!(
        balances(&rig.tw, &PrincipalId::Object(pilot)),
        balances_before,
        "no escrow"
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+credit"),
        credit_before,
        "+credit unchanged"
    );
    assert!(drain(&mut rig.tw).is_empty(), "nothing staged");
}

/// **Lens 3: capacity refusals charge nothing.** 86 t of ice into an empty hold
/// (volume); at Stickney 101 t of alloys (mass); the vertex's concentrate aboard, then
/// one tonne past its ice (both) — every quantity computed from data.
#[test]
fn ic8_capacity_refusals_charge_nothing() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    top_up(&mut rig.tw, pilot, 1_000_000_000);
    let ice = listing_on(&rig.tw, "ceres_port", "occator", "volatiles").expect("listed");
    refused_with(
        &mut rig,
        pilot,
        ice,
        86,
        "86 t of water ice packs to 120.4 m³; the hold takes 120 m³. 85 t fits.",
    );
    // At Stickney: the hull re-placed, the pilot at P08.
    let hull = ObjId(row.hull);
    let stickney_domain = port_map(&rig.tw, "stickney_port").domain.expect("a domain");
    let ceres_domain = port_map(&rig.tw, "ceres_port").domain.expect("a domain");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/domain #{}={stickney_domain}", hull.0),
    );
    assert!(out.contains("Placed"), "{out}");
    let p08 = station_room(&rig.tw, "stickney_port", "room.p08_exchange");
    teleport(&mut rig.tw, pilot, p08);
    let alloys =
        listing_on(&rig.tw, "stickney_port", "kasei", "structural_alloys").expect("listed");
    refused_with(
        &mut rig,
        pilot,
        alloys,
        101,
        "101 t of structural alloy stock weighs past the hold's 100 t rating (30.3 m³). 100 t fits.",
    );
    // Back at Ceres: the vertex's concentrate, then one tonne past its ice.
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/domain #{}={ceres_domain}", hull.0),
    );
    assert!(out.contains("Placed"), "{out}");
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, pilot, exchange);
    let goods = tacsim::definitions::goods::goods_table().expect("goods");
    let pack = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray");
    let Some(PackEntry::Unit(spec)) = pack.entry(&cid(tacsim::definitions::content::DRAY)) else {
        panic!("the Dray");
    };
    let logistics = spec.logistics.as_ref().expect("logistics");
    let cargo = tacsim::port::capacity::Compartment::from_stowage(
        &cid("cargo"),
        &logistics.stowage[&cid("cargo")],
    );
    let (d, b) = vertex::both_limits_vertex(&goods, &cargo, &cid("metals"), &cid("volatiles"))
        .expect("an integral vertex");
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    let _ = buy_and_land(&mut rig, pilot, metals, d);
    let litres = |good: &str| goods.good(&cid(good)).expect("a good").measures[&cid("si.volume_l")];
    let after = d * litres("metals") + (b + 1) * litres("volatiles");
    let line = format!(
        "{} t of water ice weighs past the hold's 100 t rating and packs to {} m³; the hold \
         takes 120 m³. {b} t fits.",
        b + 1,
        tacsim::port::capacity::format_m3(after)
    );
    refused_with(&mut rig, pilot, ice, b + 1, &line);
}

// ---------------------------------------------------------------------------
// 7. The venue gate
// ---------------------------------------------------------------------------

/// **Lens 4: the venue gate refuses Limbo and a hull at another port**, in its own
/// words: a quote and an accept from Limbo; a no-argument and a `to=<hold>` accept at
/// C07 while the hull is berthed at Stickney.
#[test]
fn ic8_the_venue_gate_refuses_limbo_and_a_hull_at_another_port() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    let ceres = port_def(LANE_JSON, "ceres_port");
    let stickney = port_def(LANE_JSON, "stickney_port");
    let ceres_name = tacsim::port::coop::port_name(&ceres);
    let not_in_port = NOT_IN_PORT.replace("{port}", &ceres_name);
    // A quote from C07, then the accept from Limbo.
    let quoted = run(&mut rig.tw, pilot, &format!("+market/quote {metals}=1"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, pilot);
    teleport(&mut rig.tw, pilot, LIMBO);
    let out = run(&mut rig.tw, pilot, &format!("+market/quote {metals}=1"));
    assert!(out.contains(&not_in_port), "{out}");
    let out = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    assert!(out.contains(&not_in_port), "{out}");
    assert!(drain(&mut rig.tw).is_empty());
    // The hull at Stickney, the pilot at C07.
    let hull = ObjId(row.hull);
    let domain = port_map(&rig.tw, "stickney_port").domain.expect("a domain");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/domain #{}={domain}", hull.0),
    );
    assert!(out.contains("Placed"), "{out}");
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, pilot, exchange);
    let elsewhere = HOLD_ELSEWHERE
        .replace("{name}", "Salt Moth")
        .replace("{other}", &tacsim::port::coop::port_name(&stickney))
        .replace("{port}", &ceres_name);
    let hold = hold_vault(&rig.tw, &row);
    for accept in [
        format!("+market/accept {quote}"),
        format!("+market/accept {quote} to={}", hold.0),
    ] {
        let out = run(&mut rig.tw, pilot, &accept);
        assert!(out.contains(&elsewhere), "{accept}: {out}");
    }
    assert!(
        market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.contract.is_none())
    );
    assert!(drain(&mut rig.tw).is_empty());
}

/// **`Presence::Ship` and the gate's other refusals** (doc 18 §4.3 "Presence"; §5:
/// IC11's `Ship` arm is "built and unit-tested here"; lead ruling L10). Two pilots
/// hold charters at Ceres. `Ship(own hull)` answers the hull's hold and admits it
/// docked at the venue with the asker anywhere (Limbo); `Ship(another's hull)` answers
/// no endpoint and is refused `NOT_YOUR_CHARTER` (so is `Ship(own hull)` naming the
/// other pilot's hold); a player's `to=` accept into another pilot's hold gets the
/// market's existence-hiding line first. The hull placed in a domain no port records is refused
/// `HOLD_NOT_HERE`, and the hull on a journey `HOLD_UNDER_WAY` — through the gate
/// with `Ship` and through a no-argument accept at C07 alike.
#[test]
fn ic8_the_ship_presence_and_the_gate_refusals() {
    let mut rig = stocked();
    let (me, row) = chartered(&mut rig, "Pilot", 1);
    let other = pilot(&mut rig, "Other", 2);
    let out = run(&mut rig.tw, other, "+coop/enrol Grey Heron");
    assert!(out.contains("now the Grey Heron"), "{out}");
    let other_charter = tacsim::port::live_charter(rig.tw.world(), other).expect("a charter");
    let other_row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == other_charter.hull.0)
        .expect("the other hull's row");
    let hull = ObjId(row.hull);
    let other_hull = ObjId(other_row.hull);
    let hold = EndpointRef(hold_vault(&rig.tw, &row));
    let other_hold = EndpointRef(hold_vault(&rig.tw, &other_row));
    let ceres = mush_market::MarketId::new("ceres").expect("venue");
    let ceres_name = tacsim::port::coop::port_name(&port_def(LANE_JSON, "ceres_port"));
    // The Ship arm, docked at the venue, the asker in Limbo.
    teleport(&mut rig.tw, me, LIMBO);
    {
        let w = rig.tw.world();
        assert_eq!(
            PortVenueGate.endpoint_for(w, me, &ceres, Presence::Ship(hull)),
            Some(hold),
            "the charter hull's hold"
        );
        assert_eq!(
            PortVenueGate.endpoint_for(w, me, &ceres, Presence::Ship(other_hull)),
            None,
            "never another pilot's hull"
        );
        assert_eq!(
            PortVenueGate
                .admit(w, me, &ceres, None, Presence::Ship(hull))
                .map_err(|r| r.line),
            Ok(())
        );
        assert_eq!(
            PortVenueGate
                .admit(w, me, &ceres, Some(hold), Presence::Ship(hull))
                .map_err(|r| r.line),
            Ok(())
        );
        for (endpoint, ship) in [(None, other_hull), (Some(other_hold), hull)] {
            assert_eq!(
                PortVenueGate
                    .admit(w, me, &ceres, endpoint, Presence::Ship(ship))
                    .map_err(|r| r.line),
                Err(NOT_YOUR_CHARTER.to_owned()),
                "{endpoint:?} as {ship:?}"
            );
        }
    }
    // `to=` another pilot's hold, at C07: the market's own custody rule answers first,
    // with the one existence-hiding line (a hold the asker may not take from is not
    // named back to them); the gate's NOT_YOUR_CHARTER is its answer only where
    // custody does not decide (the `Ship` cases above).
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, me, exchange);
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    let quoted = run(&mut rig.tw, me, &format!("+market/quote {metals}=1"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, me);
    let out = run(
        &mut rig.tw,
        me,
        &format!("+market/accept {quote} to={}", other_hold.vault().0),
    );
    assert!(out.contains(mush_market::NO_SUCH_QUOTE), "{out}");
    assert!(
        !out.contains("Grey Heron"),
        "the other hold is not named: {out}"
    );
    // The hull in a domain no port records: not berthed here.
    let open = rig
        .tw
        .world_mut()
        .create(ObjKind::Room, "Open Space", GOD, None)
        .expect("a room");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@behavior/add #{}=sim.domain", open.0),
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", open.0),
    );
    let domain = rig
        .tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .and_then(|s| s.domain_of_host(open).map(|d| d.id))
        .unwrap_or_else(|| panic!("the open domain: {out}"));
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/domain #{}={}", hull.0, domain.0),
    );
    assert!(out.contains("Placed"), "{out}");
    let name = "Salt Moth";
    let not_here = HOLD_NOT_HERE
        .replace("{name}", name)
        .replace("{port}", &ceres_name);
    assert_eq!(
        PortVenueGate
            .admit(rig.tw.world(), me, &ceres, None, Presence::Ship(hull))
            .map_err(|r| r.line),
        Err(not_here.clone())
    );
    let out = run(&mut rig.tw, me, &format!("+market/accept {quote}"));
    assert!(out.contains(&not_here), "{out}");
    // The hull under way, back towards the Ceres gallery's domain.
    let ceres_domain = port_map(&rig.tw, "ceres_port").domain.expect("a domain");
    let gallery = rig
        .tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .and_then(|s| {
            s.domains
                .get(&mush_sim::DomainId(ceres_domain))
                .map(|d| d.host)
        })
        .expect("the Ceres domain's host");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@journey/depart #{}=ic8_fixture #{}:100", hull.0, gallery.0),
    );
    let under_way = HOLD_UNDER_WAY
        .replace("{name}", name)
        .replace("{port}", &ceres_name);
    assert_eq!(
        PortVenueGate
            .admit(rig.tw.world(), me, &ceres, None, Presence::Ship(hull))
            .map_err(|r| r.line),
        Err(under_way.clone()),
        "{out}"
    );
    let out = run(&mut rig.tw, me, &format!("+market/accept {quote}"));
    assert!(out.contains(&under_way), "{out}");
    assert!(
        market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.contract.is_none()),
        "no refused accept made a contract"
    );
    assert!(drain(&mut rig.tw).is_empty(), "nothing staged");
}

// ---------------------------------------------------------------------------
// 8. The boards
// ---------------------------------------------------------------------------

/// **Lens 5: the board counts down to SOLD OUT**, on a lane whose Exchange stocks 30 t
/// of concentrate (booted as an operator seed). C08 shows only the Co-op's fittings
/// bid; P11 and P08 both show the Yard's fittings ask; P07 shows Kasei's two bids.
#[test]
fn ic8_the_board_counts_down_to_sold_out() {
    let lane = lane_variant(3, |doc| {
        doc["entries"]["ceres_port"]["def"]["vendors"][0]["stock"][0]["qty"] =
            serde_json::json!(30);
    });
    let mut rig = boot(Roster::seeded(lane));
    stock(&mut rig);
    let (pilot, _) = chartered(&mut rig, "Pilot", 1);
    let exchange = c07(&rig.tw);
    let signatory = "Margit Søndergaard-Achebe";
    let metals_line = |left: &str| {
        format!(
            "  metal concentrate: 200.00cr/t, {left} t left (Freeport Exchange; signed {signatory})."
        )
    };
    let out = look_in(&mut rig.tw, pilot, exchange);
    assert!(board(&out).contains(&metals_line("30")), "{out}");
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    let _ = buy_and_land(&mut rig, pilot, metals, 20);
    let out = run(&mut rig.tw, pilot, "look");
    assert!(board(&out).contains(&metals_line("10")), "{out}");
    let _ = buy_and_land(&mut rig, pilot, metals, 10);
    let out = run(&mut rig.tw, pilot, "look");
    let sold_out =
        "  metal concentrate: SOLD OUT: seeded 30 t, 0 left (Freeport Exchange).".to_owned();
    assert!(board(&out).contains(&sold_out), "{out}");
    let report = status(&mut rig.tw);
    assert!(
        report.contains(
            "  ceres_port ask freeport_exchange metals: SOLD OUT: seeded 30 t, 0 left. [ready]"
        ),
        "{report}"
    );
    // C07 lines: the three asks, then the alloy bid.
    let lines = board(&out);
    assert_eq!(lines.len(), 4, "{out}");
    assert!(
        lines[1].starts_with("  water ice: 100.00cr/t, 9,600 t left (Occator"),
        "{out}"
    );
    assert!(
        lines[2].starts_with("  reaction mass: 40.00cr/t, 5,000 t left"),
        "{out}"
    );
    assert!(
        lines[3].starts_with(
            "  wanted, structural alloy stock: 520.00cr/t, 7,000 t more (Freeport Works;"
        ),
        "{out}"
    );
    // C08: only the Co-op's fittings bid.
    let c08 = station_room(&rig.tw, "ceres_port", "room.c08_chandlery");
    let out = look_in(&mut rig.tw, pilot, c08);
    let lines = board(&out);
    assert_eq!(lines.len(), 1, "{out}");
    assert!(
        lines[0].starts_with("  wanted, yard fittings, crated: 560.00cr/t, 5,400 t more"),
        "{out}"
    );
    // P11 and P08: the Yard's fittings ask.
    let ask = "  yard fittings, crated: 400.00cr/t, 5,400 t left (Stickney Yards; signed";
    for local in ["room.p11_service", "room.p08_exchange"] {
        let room = station_room(&rig.tw, "stickney_port", local);
        let out = look_in(&mut rig.tw, pilot, room);
        assert!(
            board(&out).iter().any(|l| l.starts_with(ask)),
            "{local}: {out}"
        );
    }
    // P07: Kasei's two bids.
    let p07 = station_room(&rig.tw, "stickney_port", "room.p07_receiving");
    let out = look_in(&mut rig.tw, pilot, p07);
    let lines = board(&out);
    assert_eq!(lines.len(), 2, "{out}");
    assert!(
        lines[0]
            .starts_with("  wanted, metal concentrate: 280.00cr/t, 4,000 t more (Kasei Metals;"),
        "{out}"
    );
    assert!(
        lines[1].starts_with("  wanted, water ice: 200.00cr/t, 9,600 t more (Kasei Metals;"),
        "{out}"
    );
    // A room with no board shows none.
    let c05 = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    let out = look_in(&mut rig.tw, pilot, c05);
    assert!(!out.contains(tacsim::port::board::BOARD_HEADING), "{out}");
}

// ---------------------------------------------------------------------------
// 9. The lapsed player listing
// ---------------------------------------------------------------------------

/// **Lens 7: a lapsed player listing is released by the tick**, acting as the
/// vault-owning player and never GOD; the market sweep keeps the listing until the
/// hold reads Gone; the holder is told once, after the release is verified.
#[test]
fn ic8_a_lapsed_player_listing_is_released_by_the_tick() {
    let mut rig = stocked();
    let pilot = rig
        .tw
        .world_mut()
        .create(ObjKind::Player, "Seller", GOD, Some(LIMBO))
        .expect("an offline player");
    let crate_ = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "Seller's crate", pilot, Some(pilot))
        .expect("a crate");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@vault/create Test Vault=container:#{} owner=#{}",
            crate_.0, pilot.0
        ),
    );
    let vault = economy(&rig.tw)
        .snapshot()
        .vaults()
        .find(|v| v.owner == PrincipalId::Object(pilot))
        .map(|v| v.id)
        .unwrap_or_else(|| panic!("the test vault: {out}"));
    let exchange = storefront(&rig.tw, "ceres_port", "freeport_exchange");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@inv/move {}/metals:10={}", exchange.0, vault.0),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    assert_eq!(pool(&mut rig.tw, vault, "metals"), 10);
    let vendor = port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")]
        .vendor
        .expect("a vendor");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@market/vendor stock #{vendor}=tacsim:metals price=1 qty=10 days=1 endpoint={}",
            vault.0
        ),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    let listing = market(&rig.tw)
        .lot_listing_of(EndpointRef(vault), &cid("metals"))
        .expect("the hold-backed listing");
    let expires = market(&rig.tw).listing(listing).expect("listed").expires.at;
    // Real time past the listing's deadline: the sweep keeps it; the tick releases it
    // as the holder.
    rig.tw.set_now(expires + 1);
    let _ = rig.tw.fire_due((expires + 1) * 1000);
    assert!(
        market(&rig.tw).listing(listing).is_some(),
        "the sweep keeps it"
    );
    let queued = drain(&mut rig.tw);
    let releases: Vec<&AuthoritativeSubmit> = queued
        .iter()
        .filter(|s| s.action.key().owner.as_str() == "market")
        .collect();
    assert_eq!(releases.len(), 1, "{queued:?}");
    assert_eq!(releases[0].actor, pilot, "the release is the holder's");
    assert!(queued.iter().all(|s| s.actor != GOD));
    land_all(&mut rig, &queued);
    // The release has landed but the listing still stands (no sweep yet): a fire
    // now tells nobody — the notice waits for the pull verification (fixer T7).
    let _ = fire(&mut rig);
    assert!(
        market(&rig.tw).listing(listing).is_some(),
        "no sweep has run yet"
    );
    assert!(
        tacsim::port_tick::pending_notices(rig.tw.world(), pilot).is_empty(),
        "no notice before the listing is seen gone"
    );
    // The next sweep deletes it; the fires after it tell the holder once.
    sweep(&mut rig.tw, 61);
    let _ = drain(&mut rig.tw);
    assert!(
        market(&rig.tw).listing(listing).is_none(),
        "deleted once the hold is gone"
    );
    let _ = fire(&mut rig);
    let notice = format!(
        "Your market listing #{} lapsed; the 10 of metal concentrate it held are free again.",
        listing.0
    );
    assert_eq!(
        notice,
        tacsim::port::stock::release_notice(listing.0, 10, "metal concentrate")
    );
    let pending = tacsim::port_tick::pending_notices(rig.tw.world(), pilot);
    assert_eq!(
        pending.iter().filter(|line| **line == notice).count(),
        1,
        "{pending:?}"
    );
    let _ = fire(&mut rig);
    let pending = tacsim::port_tick::pending_notices(rig.tw.world(), pilot);
    assert_eq!(
        pending.iter().filter(|line| **line == notice).count(),
        1,
        "once"
    );
    let events = rig.tw.connect(pilot);
    let seen = said(&rig.tw, &events, pilot);
    assert_eq!(seen.matches(&notice).count(), 1, "{seen}");
}

/// A hold-backed listing of `qty` concentrate drawn from the Exchange storefront into
/// a fresh container vault — owned by `owner` (`None`: staff's own, so GOD) — stocked
/// by staff through the Exchange's vendor for one day, its hold landed.
fn held_listing(rig: &mut Rig, owner: Option<ObjId>, qty: u64) -> (VaultId, ListingId) {
    let holder = owner.unwrap_or(GOD);
    let crate_ = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "A crate", holder, owner)
        .expect("a crate");
    let owner_word = owner.map_or_else(String::new, |o| format!(" owner=#{}", o.0));
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@vault/create Crate=container:#{}{owner_word}", crate_.0),
    );
    let vault = economy(&rig.tw)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::Container(crate_))
        .map(|v| v.id)
        .unwrap_or_else(|| panic!("the crate vault: {out}"));
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .vault(vault)
            .map(|v| v.owner.clone()),
        Some(PrincipalId::Object(holder))
    );
    let exchange = storefront(&rig.tw, "ceres_port", "freeport_exchange");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@inv/move {}/metals:{qty}={}", exchange.0, vault.0),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(rig, &queued);
    let vendor = port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")]
        .vendor
        .expect("a vendor");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@market/vendor stock #{vendor}=tacsim:metals price=1 qty={qty} days=1 endpoint={}",
            vault.0
        ),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(rig, &queued);
    let listing = market(&rig.tw)
        .lot_listing_of(EndpointRef(vault), &cid("metals"))
        .expect("the hold-backed listing");
    (vault, listing)
}

/// **A lapsed listing the tick cannot release never blocks a later one** (fixer F1):
/// two lower-id lapsed hold-backed listings have no holder the tick may act as — one
/// on a vault staff own (GOD), one on a vault whose player owner is gone — and a
/// player's higher-id listing lapses after them. The release arm passes over both
/// (the sweep keeps them, lead ruling L7) and releases the player's on the first
/// fire, as the player.
#[test]
fn ic8_an_unreleasable_lapsed_listing_never_blocks_a_later_one() {
    let mut rig = stocked();
    let (_, staff) = held_listing(&mut rig, None, 5);
    let ghost = rig
        .tw
        .world_mut()
        .create(ObjKind::Player, "Ghost", GOD, Some(LIMBO))
        .expect("a player");
    let (_, haunted) = held_listing(&mut rig, Some(ghost), 5);
    rig.tw
        .world_mut()
        .set_flag(ghost, mush_core::Flags::GOING, true)
        .expect("the owner is going");
    let seller = rig
        .tw
        .world_mut()
        .create(ObjKind::Player, "Seller", GOD, Some(LIMBO))
        .expect("a player");
    let (_, listing) = held_listing(&mut rig, Some(seller), 10);
    assert!(
        staff < haunted && haunted < listing,
        "the player's is last in line"
    );
    let expires = [staff, haunted, listing]
        .iter()
        .map(|id| market(&rig.tw).listing(*id).expect("listed").expires.at)
        .max()
        .expect("three");
    // Past every deadline; three fires, each of which would re-pick the head of the
    // line if the arm did not pass over what it cannot release.
    rig.tw.set_now(expires + 1);
    let _ = rig.tw.fire_due((expires + 1) * 1000);
    let mut releases: Vec<AuthoritativeSubmit> = drain(&mut rig.tw)
        .into_iter()
        .filter(|s| s.action.key().owner.as_str() == "market")
        .collect();
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        releases.extend(
            queued
                .into_iter()
                .filter(|s| s.action.key().owner.as_str() == "market"),
        );
    }
    assert_eq!(releases.len(), 1, "{releases:?}");
    assert_eq!(
        releases[0].actor, seller,
        "the player's release, as the player"
    );
    land_all(&mut rig, &releases);
    sweep(&mut rig.tw, 61);
    let _ = drain(&mut rig.tw);
    assert!(
        market(&rig.tw).listing(listing).is_none(),
        "released and swept"
    );
    assert!(
        market(&rig.tw).listing(staff).is_some(),
        "a hold no one released is kept"
    );
}

// ---------------------------------------------------------------------------
// 10–11. One predicate; the port institutions
// ---------------------------------------------------------------------------

/// **The plan is governed exactly where the gate governs** (IC7 R5): for a chartered
/// hold, an unchartered hull's stowage vault, every storefront, a player's container
/// vault, `plan(..).is_some() == governed(..)`, and a stowage vault is governed. An
/// `interior:` vault on a non-stowage hull room cannot be made at all: the airlock
/// space has member rooms and a walked room is no space, so `@vault/create` refuses
/// both (IC4's boundary rule) and makes nothing.
#[test]
fn ic8_plan_is_governed_exactly_where_the_gate_governs() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let mut endpoints: Vec<(String, VaultId)> =
        vec![("the hold".to_owned(), hold_vault(&rig.tw, &row))];
    for entry in ["ceres_port", "stickney_port"] {
        for (org, row) in port_map(&rig.tw, entry).vendors {
            endpoints.push((format!("{entry} {org}"), VaultId(row.vault)));
        }
    }
    let crate_ = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "Pilot's crate", pilot, Some(pilot))
        .expect("a crate");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@vault/create Crate=container:#{} owner=#{}",
            crate_.0, pilot.0
        ),
    );
    let other = port_map(&rig.tw, "ceres_port").hulls[1].clone();
    let interior_vault = |rig: &mut Rig, room: &str| {
        let room = ObjId(other.rooms[&cid(room)]);
        let out = run(
            &mut rig.tw,
            GOD,
            &format!("@vault/create V=interior:#{}", room.0),
        );
        (
            economy(&rig.tw)
                .snapshot()
                .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(room))
                .map(|v| v.id),
            out,
        )
    };
    let (stowage, out) = interior_vault(&mut rig, "stowage");
    let stowage = stowage.unwrap_or_else(|| panic!("a second hull's stowage vault: {out}"));
    endpoints.push(("a second hull's stowage".to_owned(), stowage));
    // Every other hull space backs no vault: the airlock is a multi-room space (its
    // walked rooms are members) and a walked room is no space of its own, so the
    // custody rule (`InteriorCustodyProvider::backing_valid`, cut IC4) refuses both and
    // no non-stowage hull endpoint can exist for the predicates to disagree on.
    let before = economy(&rig.tw).snapshot().vaults().count();
    for room in ["airlock", "hold"] {
        let (made, out) = interior_vault(&mut rig, room);
        assert!(made.is_none(), "{room}: {out}");
        assert!(
            out.contains("That object cannot back a vault."),
            "{room}: {out}"
        );
    }
    assert_eq!(economy(&rig.tw).snapshot().vaults().count(), before);
    for vault in economy(&rig.tw)
        .snapshot()
        .vaults()
        .filter(|v| v.owner == PrincipalId::Object(pilot))
        .map(|v| v.id)
        .collect::<Vec<_>>()
    {
        endpoints.push(("a pilot's vault".to_owned(), vault));
    }
    let w = rig.tw.world();
    for (label, vault) in &endpoints {
        let endpoint = EndpointRef(*vault);
        assert_eq!(
            PortCapacity.plan(w, endpoint).is_some(),
            PortVenueGate.governed(w, endpoint),
            "{label}"
        );
    }
    assert!(PortVenueGate.governed(w, EndpointRef(hold_vault(&rig.tw, &row))));
    assert!(
        PortVenueGate.governed(w, EndpointRef(stowage)),
        "an unchartered hull's stowage is a hold"
    );
    assert!(!PortVenueGate.governed(w, EndpointRef(storefront(&rig.tw, "ceres_port", "occator"))));
}

/// **Port sellers and party caps are the port institutions**: every vendor and the
/// Trust answer yes; the pilot, Ceres Clearing (no vendor), Tycho Mutual and an
/// unrelated org answer no; a Stickney vendor is no port seller at Ceres.
#[test]
fn ic8_port_sellers_and_party_caps_are_the_port_institutions() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1);
    let _ = run(&mut rig.tw, GOD, "@org/create Unrelated Holdings");
    let unrelated = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("the org");
    let w = rig.tw.world();
    let ceres_map = port_map(&rig.tw, "ceres_port");
    let stickney_map = port_map(&rig.tw, "stickney_port");
    let org = |map: &PortMap, id: &str| PrincipalId::Organization(OrgId(map.orgs[&cid(id)].org));
    let ceres = mush_market::MarketId::new("ceres").expect("venue");
    for vendor in ["freeport_exchange", "occator", "coop"] {
        assert!(
            PortVenueGate.port_seller(w, &ceres, &org(&ceres_map, vendor)),
            "{vendor}"
        );
        assert!(
            PortPartyCaps.has_cap(w, &org(&ceres_map, vendor), PartyCap::Accept),
            "{vendor}"
        );
    }
    for vendor in ["kasei", "stickney_yards", "yard_commissary"] {
        assert!(
            !PortVenueGate.port_seller(w, &ceres, &org(&stickney_map, vendor)),
            "{vendor}"
        );
        assert!(
            PortPartyCaps.has_cap(w, &org(&stickney_map, vendor), PartyCap::Accept),
            "{vendor}"
        );
    }
    assert!(
        PortPartyCaps.has_cap(w, &org(&ceres_map, "coop_trust"), PartyCap::Accept),
        "the Trust"
    );
    for no in [
        PrincipalId::Object(pilot),
        org(&ceres_map, "ceres_clearing"),
        org(&ceres_map, "tycho_mutual"),
        PrincipalId::Organization(unrelated),
    ] {
        assert!(!PortPartyCaps.has_cap(w, &no, PartyCap::Accept), "{no:?}");
        assert!(!PortVenueGate.port_seller(w, &ceres, &no), "{no:?}");
    }
}

// ---------------------------------------------------------------------------
// 12–14. IC7's gaps
// ---------------------------------------------------------------------------

/// **A Draw accept is refused on storefront availability** (an IC7 gap): staff move
/// stock out from under the listing; the accept refuses before money moves, the
/// Listings line is Flagged with the drift, and the arm pages it once.
#[test]
fn ic8_a_draw_accept_is_refused_on_storefront_availability() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1);
    let staff_box = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "Staff box", GOD, None)
        .expect("a box");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@vault/create Box=container:#{}", staff_box.0),
    );
    let boxed = economy(&rig.tw)
        .snapshot()
        .vaults()
        .map(|v| v.id)
        .max()
        .expect("the box vault");
    let exchange = storefront(&rig.tw, "ceres_port", "freeport_exchange");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@inv/move {}/metals:3995={}", exchange.0, boxed.0),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    assert_eq!(pool(&mut rig.tw, exchange, "metals"), 5);
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    let before = balances(&rig.tw, &PrincipalId::Object(pilot));
    let (accepted, quote) = buy(&mut rig, pilot, metals, 10);
    // The market's Draw pre-check answers its undeliverable line both for a short
    // storefront and for no fact provider; the facts line reads ready, so it is the
    // storefront's availability that refused (fixer T8).
    assert!(
        accepted.contains(
            "That cannot be delivered to you right now, so the sale was not made. Nothing was \
             charged."
        ),
        "refused on the storefront's stock: {accepted}"
    );
    assert!(
        status(&mut rig.tw).contains("market facts: answered, so drawn sales can settle. [ready]"),
        "the facts are answered"
    );
    assert!(
        market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.contract.is_none()),
        "{accepted}"
    );
    assert_eq!(
        market(&rig.tw).listing(metals).expect("listed").quantity,
        4_000,
        "the listing is untouched"
    );
    assert!(drain(&mut rig.tw).is_empty(), "nothing staged");
    assert_eq!(
        balances(&rig.tw, &PrincipalId::Object(pilot)),
        before,
        "no money moved"
    );
    let report = status(&mut rig.tw);
    assert!(
        report.contains(&format!(
            "  ceres_port ask freeport_exchange metals: listing #{metals} lists 4000 t but the \
             storefront holds 5 t. [flagged]"
        )),
        "{report}"
    );
    let _ = fire(&mut rig);
    let _ = fire(&mut rig);
    let pages = tacsim::port_tick::drift_pages(rig.tw.world());
    assert_eq!(
        pages
            .iter()
            .filter(|(vault, good)| *vault == exchange.0 && good.as_str() == "metals")
            .count(),
        1,
        "{pages:?}"
    );
}

/// **The draw mirror keeps up with more than 64 undrawn Draws** (an IC7 gap): 72
/// accepted Draws across nine buyers and the three Ceres vendors, each landed into the
/// buyer's own crate, are all
/// mirrored within two market sweeps.
#[test]
fn ic8_reconcile_draws_mirrors_more_than_64_undrawn_draws() {
    let mut rig = stocked();
    let metals = listing_on(&rig.tw, "ceres_port", "freeport_exchange", "metals").expect("listed");
    let ice = listing_on(&rig.tw, "ceres_port", "occator", "volatiles").expect("listed");
    let provisions = listing_on(&rig.tw, "ceres_port", "coop", "provisions").expect("listed");
    let exchange = c07(&rig.tw);
    let mut quotes = Vec::new();
    for n in 0..9 {
        let buyer = rig.tw.spawn_player(&format!("Buyer{n}"));
        teleport(&mut rig.tw, buyer, exchange);
        let _ = run(
            &mut rig.tw,
            GOD,
            &format!("@account/open #{}={}", buyer.0, tacsim::CREDIT),
        );
        top_up(&mut rig.tw, buyer, 1_000_000);
        let crate_ = rig
            .tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Crate {n}"), buyer, Some(buyer))
            .expect("a crate");
        let _ = run(
            &mut rig.tw,
            GOD,
            &format!(
                "@vault/create Crate{n}=container:#{} owner=#{}",
                crate_.0, buyer.0
            ),
        );
        let vault = economy(&rig.tw)
            .snapshot()
            .vaults()
            .find(|v| v.owner == PrincipalId::Object(buyer))
            .map(|v| v.id)
            .expect("the crate vault");
        // Spread across the three Ceres vendors: an org stands on at most
        // `MAX_LIVE_CONTRACTS_PER_PARTY` (32) live contracts at once.
        for listing in [
            metals, metals, metals, ice, ice, ice, provisions, provisions,
        ] {
            let quoted = run(&mut rig.tw, buyer, &format!("+market/quote {listing}=1"));
            assert!(quoted.contains("Quote #"), "{quoted}");
            let quote = newest_quote(&rig.tw, buyer);
            let out = run(
                &mut rig.tw,
                buyer,
                &format!("+market/accept {quote} to={}", vault.0),
            );
            let queued = drain(&mut rig.tw);
            assert_eq!(queued.len(), 1, "{out}");
            land_all(&mut rig, &queued);
            quotes.push(quote);
        }
    }
    assert!(quotes.len() >= 65);
    // Before any mirror, the landed Draws are in hand-over on the Listings line: the
    // listing and the storefront have both fallen by the 27 t drawn (fixer T13).
    let exchange_vault = storefront(&rig.tw, "ceres_port", "freeport_exchange");
    let listed = market(&rig.tw).listing(metals).expect("listed").quantity;
    let held = pool(&mut rig.tw, exchange_vault, "metals");
    assert_eq!((listed, held), (4_000 - 27, 4_000 - 27));
    let report = status(&mut rig.tw);
    let line = format!(
        "  ceres_port ask freeport_exchange metals: listing #{metals} at 200.00cr/t, {listed} t \
         listed; the storefront holds {held} t, 27 t in hand-over. [ready]"
    );
    assert!(report.contains(&line), "want `{line}`:\n{report}");
    let drawn = |tw: &TestWorld| {
        quotes
            .iter()
            .filter(|q| {
                market(tw)
                    .quote(**q)
                    .is_some_and(|q| q.drawn_at.is_some() || q.settled.is_some())
            })
            .count()
    };
    assert_eq!(drawn(&rig.tw), 0, "nothing mirrored before a sweep");
    sweep(&mut rig.tw, 61);
    let _ = drain(&mut rig.tw);
    sweep(&mut rig.tw, 61);
    let _ = drain(&mut rig.tw);
    assert_eq!(
        drawn(&rig.tw),
        quotes.len(),
        "every Draw mirrored within two sweeps"
    );
    assert_eq!(refused_settlements(&rig.tw), 0);
}

/// **`+market/depth` defaults to the venue of the room** (an IC7 gap): reaction mass
/// is listed at both ports (Occator at Ceres, Kasei at Stickney), so the market's own
/// fallback — the venue of the first public listing — would answer one venue for both
/// rooms; the room's venue answers `ceres` at C07 and `stickney` at P08.
#[test]
fn ic8_depth_defaults_to_the_venue_of_the_room() {
    let mut rig = stocked();
    assert!(listing_on(&rig.tw, "ceres_port", "occator", "propellant").is_some());
    assert!(listing_on(&rig.tw, "stickney_port", "kasei", "propellant").is_some());
    let who = rig.tw.spawn_player("Looker");
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, who, exchange);
    let out = run(&mut rig.tw, who, "+market/depth tacsim:propellant");
    assert_eq!(
        out.lines().next(),
        Some("tacsim:propellant in ceres"),
        "{out}"
    );
    let p08 = station_room(&rig.tw, "stickney_port", "room.p08_exchange");
    teleport(&mut rig.tw, who, p08);
    let out = run(&mut rig.tw, who, "+market/depth tacsim:propellant");
    assert_eq!(
        out.lines().next(),
        Some("tacsim:propellant in stickney"),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// 15. The standing bids
// ---------------------------------------------------------------------------

/// **The standing bids are posted, and read "not buying" until the capital settles**,
/// on `+market/list` and on the boards alike; before the first tick an ask's board
/// line reads "not yet on the board"; a quote on a bid is refused with
/// `WANTED_IS_A_BID`.
#[test]
fn ic8_wanted_bids_are_posted_and_render_not_buying_when_unfunded() {
    let mut rig = boot_unsettled(Roster::bare());
    let who = rig.tw.spawn_player("Looker");
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, who, exchange);
    let bid = port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].bids
        [&cid("structural_alloys")]
        .listing;
    let row = format!(
        "#{bid} structural alloy stock — wanted 7000 × 520.00cr [ceres] by Freeport Exchange"
    );
    let list = run(&mut rig.tw, who, "+market/list");
    assert!(list.contains(&format!("{row} — not buying")), "{list}");
    // The boards before the capital settles and before any tick (fixer T10).
    let out = run(&mut rig.tw, who, "look");
    let lines = board(&out);
    assert_eq!(
        lines,
        vec![
            "  metal concentrate: not yet on the board (Freeport Exchange).".to_owned(),
            "  water ice: not yet on the board (Occator Water & Propellant).".to_owned(),
            "  reaction mass: not yet on the board (Occator Water & Propellant).".to_owned(),
            "  wanted, structural alloy stock: not buying (Freeport Works).".to_owned(),
        ],
        "{out}"
    );
    let p07 = station_room(&rig.tw, "stickney_port", "room.p07_receiving");
    let out = look_in(&mut rig.tw, who, p07);
    assert_eq!(
        board(&out),
        vec![
            "  wanted, metal concentrate: not buying (Kasei Metals).".to_owned(),
            "  wanted, water ice: not buying (Kasei Metals).".to_owned(),
        ],
        "{out}"
    );
    teleport(&mut rig.tw, who, exchange);
    sweep(&mut rig.tw, 120);
    let list = run(&mut rig.tw, who, "+market/list");
    let line = list
        .lines()
        .find(|l| l.contains(&row))
        .unwrap_or_else(|| panic!("{list}"));
    assert!(!line.contains("not buying"), "{line}");
    let out = run(&mut rig.tw, who, "look");
    assert!(
        board(&out).contains(
            &"  wanted, structural alloy stock: 520.00cr/t, 7,000 t more (Freeport Works; signed \
              Margit Søndergaard-Achebe)."
                .to_owned()
        ),
        "{out}"
    );
    let out = run(&mut rig.tw, who, &format!("+market/quote {bid}=1"));
    assert!(out.contains(mush_market::WANTED_IS_A_BID), "{out}");
}

// ---------------------------------------------------------------------------
// Retire (item 4; fixer F2 and T15)
// ---------------------------------------------------------------------------

/// **Retire refuses a deal in progress, withdraws only the vendors' Draws and bids,
/// and a later apply posts the bids afresh** (doc 18 §4.7 "Retire withdraws asks and
/// bids"; IC8 item 4; fixer F2). A lane that drops `ceres_port` is refused while a
/// buyer's Draw from the Exchange is unsettled, naming the vendor; once it settles the
/// retire applies: every Ceres Draw listing and standing bid is withdrawn, a
/// staff-stocked hold-backed listing of the Exchange's stands (withdrawing it would
/// stage a release from the applier), and the withdrawn bids leave the map. A later
/// version that brings the port back posts every bid again as a live, funded standing
/// bid — never "not buying; its intake is taken".
#[test]
#[allow(clippy::too_many_lines)]
fn ic8_retire_withdraws_draws_and_bids_and_a_later_apply_posts_the_bids_afresh() {
    let retired = mush_content::load_pack(&lane_variant(4, |doc| {
        doc["entries"]
            .as_object_mut()
            .expect("the entries")
            .remove("ceres_port");
    }))
    .expect("the retiring variant verifies");
    let mut rig = boot(Roster::bare().with_shelf(vec![retired]));
    stock(&mut rig);
    let (_, held) = held_listing(&mut rig, None, 5);
    let ceres = port_map(&rig.tw, "ceres_port");
    let draws: Vec<ListingId> = [
        ("freeport_exchange", "metals"),
        ("occator", "volatiles"),
        ("occator", "propellant"),
        ("coop", "provisions"),
    ]
    .iter()
    .map(|(org, good)| listing_on(&rig.tw, "ceres_port", org, good).expect("listed"))
    .collect();
    let bids: Vec<ListingId> = ceres
        .vendors
        .values()
        .flat_map(|row| row.bids.values().map(|b| ListingId(b.listing)))
        .collect();
    assert_eq!(bids.len(), 2, "the alloy and fittings bids");
    // A buyer's Draw from the Exchange, accepted into their own crate and unsettled.
    let buyer = rig.tw.spawn_player("Buyer");
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, buyer, exchange);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}={}", buyer.0, tacsim::CREDIT),
    );
    top_up(&mut rig.tw, buyer, 1_000_000);
    let crate_ = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "Buyer's crate", buyer, Some(buyer))
        .expect("a crate");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@vault/create Crate=container:#{} owner=#{}",
            crate_.0, buyer.0
        ),
    );
    let vault = economy(&rig.tw)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::Container(crate_))
        .map(|v| v.id)
        .expect("the crate vault");
    let quoted = run(&mut rig.tw, buyer, &format!("+market/quote {}=1", draws[0]));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, buyer);
    let out = run(
        &mut rig.tw,
        buyer,
        &format!("+market/accept {quote} to={}", vault.0),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    let (_, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains("ceres_port: vendor freeport_exchange has a deal in progress"),
        "{apply}"
    );
    assert!(!apply.contains("Applied "), "{apply}");
    assert!(
        draws
            .iter()
            .chain(&bids)
            .all(|l| market(&rig.tw).listing(*l).is_some())
    );
    // Settle it, then retire.
    let contract = market(&rig.tw)
        .quote(quote)
        .and_then(|q| q.contract)
        .expect("a contract");
    for _ in 0..6 {
        sweep(&mut rig.tw, 61);
        let _ = drain(&mut rig.tw);
        if market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.settled.is_some())
        {
            break;
        }
    }
    assert!(
        matches!(
            economy(&rig.tw)
                .snapshot()
                .contract(contract)
                .map(|c| c.state),
            Some(ContractState::Settled { .. })
        ),
        "the Draw settles"
    );
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@4")) && apply.contains("1 retired"),
        "{plan}\n{apply}"
    );
    for listing in draws.iter().chain(&bids) {
        assert!(
            market(&rig.tw).listing(*listing).is_none(),
            "#{listing} withdrawn"
        );
    }
    assert!(
        market(&rig.tw).listing(held).is_some(),
        "a hold-backed vendor listing stands"
    );
    let after = port_map(&rig.tw, "ceres_port");
    assert!(after.clerk.is_some(), "the map is kept");
    assert!(
        after.vendors.values().all(|row| row.bids.is_empty()),
        "the withdrawn bids left the map: {:?}",
        after.vendors
    );
    // A later version brings the port back: the bids are posted afresh.
    let back = mush_content::load_pack(&lane_variant(5, |_| {})).expect("the variant verifies");
    rig.roster.shelf = Some(vec![back]);
    let mut rig = reboot(rig);
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@5")),
        "{plan}\n{apply}"
    );
    let now = port_map(&rig.tw, "ceres_port");
    let reposted: Vec<ListingId> = now
        .vendors
        .values()
        .flat_map(|row| row.bids.values().map(|b| ListingId(b.listing)))
        .collect();
    assert_eq!(reposted.len(), 2, "both bids recorded again");
    for listing in &reposted {
        assert!(
            !bids.contains(listing),
            "a fresh listing, not the withdrawn one"
        );
        assert!(
            market(&rig.tw)
                .listing(*listing)
                .is_some_and(|l| l.source.is_wanted()),
            "#{listing} stands as a bid"
        );
    }
    let report = status(&mut rig.tw);
    assert!(!report.contains("its intake is taken"), "{report}");
    let alloys = now.vendors[&cid("freeport_exchange")].bids[&cid("structural_alloys")].listing;
    assert!(
        report.contains(&format!(
            "  ceres_port bid freeport_exchange structural_alloys: listing #{alloys} at \
             520.00cr/t, 7000 of 7000 t wanted. [ready]"
        )),
        "{report}"
    );
}

// ---------------------------------------------------------------------------
// 16–17. Updates by the shelf
// ---------------------------------------------------------------------------

/// **A world on lane v2 takes v3 by the shelf.** The published v2 (references, no
/// vendors) boots as an operator seed; a stale `lock` alias and a stale hull-room
/// detail are planted; `@atlas/pack plan`/`apply` takes the shipped v3, whose update
/// path makes the vendors, the clerks, the vaults, the boards and the bids and
/// re-dresses every hull; then the pools seed.
#[test]
fn ic8_a_world_on_lane_v2_takes_v3_by_the_shelf() {
    let v2 = lane_variant(2, |doc| {
        for entry in ["ceres_port", "stickney_port"] {
            if let Some(def) = doc["entries"][entry]["def"].as_object_mut() {
                def.remove("vendors");
            }
        }
    });
    assert_eq!(
        mush_content::parse_pack(&v2)
            .expect("v2 parses")
            .manifest()
            .digest()
            .to_hex(),
        LANE_V2_DIGEST,
        "the rebuilt bytes are the published v2"
    );
    let rig = boot(Roster::seeded(v2));
    for entry in ["ceres_port", "stickney_port"] {
        let map = port_map(&rig.tw, entry);
        assert!(
            map.clerk.is_none() && map.vendors.is_empty(),
            "v2 makes no vendors"
        );
    }
    // Plant the stale state on the first hull: an alias and a room detail.
    let mut rig = rig;
    let row = port_map(&rig.tw, "ceres_port").hulls[0].clone();
    let airlock_exit = ObjId(row.exits[1]);
    rig.tw
        .world_mut()
        .add_alias(airlock_exit, "lock")
        .expect("the stale alias");
    let passage = ObjId(row.rooms[&cid("passage")]);
    let attr = format!(
        "{}{}",
        tacsim::port::hull::HULL_DETAIL_PREFIX,
        mush_core::fold_detail_keyword("manifest slate")
    );
    rig.tw
        .world_mut()
        .set_attr(passage, &attr, mush_core::Attr::text("stale", GOD))
        .expect("the stale detail");
    // Reboot into the production roster, which ships v3, and take it by the shelf.
    let Rig {
        mut tw,
        oracle,
        typed,
        ..
    } = rig;
    let folded = fold_module_ops(tw.take_module_ops());
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
        apply.contains(&format!("Applied {LANE_PACK}@3")),
        "{plan}\n{apply}"
    );
    for entry in ["ceres_port", "stickney_port"] {
        let map = port_map(&rig.tw, entry);
        assert!(map.clerk.is_some(), "{entry}: the clerk");
        assert_eq!(map.vendors.len(), 3, "{entry}: the vendors");
        assert!(!map.boards.is_empty(), "{entry}: the boards");
        for row in map.vendors.values() {
            assert!(row.vendor.is_some() && row.grant.is_some());
            for bid in row.bids.values() {
                assert!(market(&rig.tw).listing(ListingId(bid.listing)).is_some());
            }
        }
    }
    let want = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray");
    let Some(PackEntry::Unit(spec)) = want.entry(&cid(tacsim::definitions::content::DRAY)) else {
        panic!("the Dray");
    };
    let interior = spec.interior.as_ref().expect("the interior");
    let slate = interior
        .rooms
        .iter()
        .find(|r| r.id.as_str() == "passage")
        .and_then(|r| r.details.get("manifest slate"))
        .expect("the slate")
        .clone();
    for row in port_map(&rig.tw, "ceres_port").hulls {
        let exit = rig
            .tw
            .world()
            .get(ObjId(row.exits[1]))
            .expect("the airlock exit")
            .clone();
        assert!(
            !exit.aliases.iter().any(|a| a.eq_ignore_ascii_case("lock")),
            "{}: {:?}",
            row.mark,
            exit.aliases
        );
        let passage = ObjId(row.rooms[&cid("passage")]);
        let text = rig
            .tw
            .world()
            .attr(passage, &attr)
            .and_then(|a| a.as_text().map(str::to_owned))
            .unwrap_or_default();
        assert_eq!(text, slate, "{}: the detail is restored", row.mark);
    }
    stock(&mut rig);
    assert_eq!(receipts(&mut rig.tw).len(), 8);
    no_refused_writes(&rig.tw);
}

/// **An ask re-price withdraws the listing and the arm relists** at the new price: a
/// shelf variant moves Occator's reaction mass from 40 to 41; the update withdraws the
/// old Draw listing, and the next arm visit lists it at 41.00cr.
#[test]
fn ic8_an_ask_reprice_withdraws_and_the_arm_relists() {
    let v4_json = lane_variant(4, |doc| {
        doc["entries"]["ceres_port"]["def"]["vendors"][1]["asks"][1]["price_major"] =
            serde_json::json!(41);
    });
    let v4 = mush_content::load_pack(&v4_json).expect("the variant verifies");
    // On the shelf for `@atlas/pack`, and loaded by the layer so the report and the
    // stock arm read its asks once it is latched.
    let mut roster = Roster::bare().with_shelf(vec![v4]);
    roster.extra_lane = Some(v4_json);
    let mut rig = boot(roster);
    stock(&mut rig);
    let old = listing_on(&rig.tw, "ceres_port", "occator", "propellant").expect("listed");
    assert_eq!(
        market(&rig.tw).listing(old).expect("listed").price.minor(),
        4_000
    );
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@4")),
        "{plan}\n{apply}"
    );
    assert!(
        market(&rig.tw).listing(old).is_none(),
        "the old listing is withdrawn"
    );
    for _ in 0..4 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    let new = listing_on(&rig.tw, "ceres_port", "occator", "propellant").expect("relisted");
    assert_ne!(new, old);
    assert_eq!(
        market(&rig.tw).listing(new).expect("listed").price.minor(),
        4_100
    );
}

/// A test shelf with the four station packs ahead of it (doc 18 cut IC8): the
/// `PortApplier` resolves the vendors' storefront items and signatories from the
/// station packs on the atlas shelf (lead ruling L3), as the production shelf always
/// carries them.
fn with_stations(shelf: &[mush_content::Pack]) -> Vec<mush_content::Pack> {
    let mut full = tacsim::definitions::content::station_packs().expect("the station packs");
    full.extend(shelf.iter().cloned());
    full
}
