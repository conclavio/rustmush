//! **Doc 20 cut SM.10 acceptance — claims** (doc 20 §1 beat 10, §9.2–§9.6, §10, §11; D-20-12;
//! the SM.10 brief's §3.11, C1–C23).
//!
//! A licensed pilot's rig stands on an open block the pilot has read by hand; `+claim/stake`
//! previews (their readings, the soundings heard, any cooling term, the fee) and `go` offers and
//! escrows a Public fee contract and submits ONE `industry_claim_tenure` under the site's issuer,
//! which files a claim over every opened deposit of the block (possibly none) and stages
//! tacsim's own block record in the same bundle; the site clerk accepts the fee for the Registry
//! on the port tick; the economy settles it and the chronicle records it. Renewals, releases,
//! lapses (the clerk's, at the term's end), take-overs, contests, the cooling term, the cap, the
//! paper and the Registrar's rule follow.
//!
//! Composed on the composition root (`mining.rs`'s rig, copied: the per-file precedent; the
//! shared pieces — the strip, the account oracle and the three geology tables — are
//! `support::claims`), **booted keyed**, the Ceres table replaced by one of the three
//! deposit-count tables where a test names it (`world_with_table`, reviewer F10: the per-block
//! opened-deposit count asserted from the opening plan, so no test passes vacuously), at world
//! rate 1,000 so a 168 IC day term passes in four real hours of `fire_due`. Every act goes
//! through the real verb, the real offer route, the land path and the ticks; rigs are placed by
//! the tick-batch placement seam (`mush_sim::place`), never a fake drive.

use crate::support;

use crate::geology_keyed;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use mush_core::{
    AccountId, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId, CommitToken, GOD,
    IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext, OfferSubmission,
    RequestUid, ScopedIdempotencyKey, World,
};
use mush_economy::{ContractId, ContractState, EconomyConfig, EconomyStore, VaultId};
use mush_industry::{ClaimRecord, ClaimSubject, IndustryFamilies, IndustryHandles, TenureRecord};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::claims as cl;
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::claims::{BlockKey, BlockTenure, PaperState};
use tacsim::surface::geology::{BodyGeology, GeologyTable, GeologyTableWire, OreFamily};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::opening::{OpeningRow, opening_plan};
use tacsim::surface::{ChartKey, ChartRecord, GeologyBoot, GeologyState, RigKey, RigRecord};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{KEY_1, secret};
use support::claims::{ALL_BARREN, ALL_RICH, METALS_ONLY, MapOracle, install_oracle};

/// The ramp's cell.
const RAMP_CELL: (i64, i64) = (128, 124);
/// Ahuna Flank's port entry.
const AHUNA: &str = "ahuna_port";
/// Occator Rim's port entry.
const OCCATOR: &str = "occator_port";
/// The Freeport Registry's name.
const REGISTRY: &str = "Freeport Registry";
/// The world rate the claim suites run at (IC seconds a real second).
const RATE: u32 = 1_000;
/// One IC day.
const DAY: i64 = 86_400;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (mining.rs's, booted keyed)
// ---------------------------------------------------------------------------

/// The composition: `commerce.rs`'s bare roster (no sheet, the test world's layer, the
/// shipped game content).
#[derive(Clone, Copy, Default)]
struct Roster;

impl Roster {
    fn modules(self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let game = tacsim::GameEconomyConfig::default();
        let cfg = TacsimConfig {
            enabled: true,
            ..TacsimConfig::default()
        };
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.push(comp::tacsim_test_world_module_wired(cfg, true, inventory));
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
        mods
    }
}

/// A booted world, its oracle, the cumulative store, the typed post-images, the ledger and the
/// table it runs on.
struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed: BTreeMap<(SmolStr, SmolStr), Value>,
    ledger: std::collections::BTreeSet<String>,
    table: Option<&'static str>,
    /// The real instant the world booted at.
    start: i64,
}

fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

fn with_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x63; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// Install `table` as Ceres's geology (test data), before any site opens.
fn install_table(tw: &mut TestWorld, table: &str) {
    let wire: GeologyTableWire = serde_json::from_str(table).expect("the table parses");
    let table = GeologyTable::from_wire(&wire).expect("the table checks");
    tacsim::surface::install_body_geology(
        tw.world_mut(),
        BodyGeology::new(cid("ceres"), geology_keyed::CERES_RADIUS_KM, table).expect("ceres"),
    )
    .expect("installed");
}

/// **Compose over `world`, hand the key in, then run the boot pass over `store`**
/// (`build_engine`'s order), the table (if any) installed over the lane's.
fn boot_over(
    world: World,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    table: Option<&str>,
) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, Roster.modules())
        .expect("the roster composes");
    assert!(tacsim::surface::install_geology(
        &mut engine.world,
        GeologyBoot::Key(secret(KEY_1))
    ));
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(store, &mut out, &mut fx)
        .expect("the world boots");
    let mut tw = TestWorld::from_engine(engine);
    if let Some(table) = table {
        install_table(&mut tw, table);
    }
    no_refused_writes(&tw);
    tw
}

/// **Boot keyed with `table` over Ceres** (or the lane's own), settle and stock every port, at
/// [`RATE`].
fn world_on(table: Option<&'static str>) -> Rig {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_test_writer()
        .try_init();
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), table);
    let start = tw.world().now();
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    let mut rig = Rig {
        tw,
        oracle,
        store: HashMap::new(),
        typed: BTreeMap::new(),
        ledger: std::collections::BTreeSet::new(),
        table,
        start,
    };
    sweep(&mut rig.tw, 120);
    stock(&mut rig);
    set_rate(&mut rig.tw, RATE);
    rig
}

/// Fold every write since the first boot and reboot keyed into the same roster; every connection
/// drops and `reconnect` come back.
fn reboot(rig: Rig, reconnect: &[ObjId]) -> Rig {
    let Rig {
        mut tw,
        oracle,
        mut store,
        typed,
        ledger,
        table,
        start,
    } = rig;
    fold_module_ops_into(&mut store, tw.take_module_ops());
    let mut world = tw.into_engine().world;
    let connected: Vec<ObjId> = world
        .iter()
        .map(|o| o.id)
        .filter(|id| world.is_connected(*id))
        .collect();
    for id in connected {
        world.set_connected(id, false);
    }
    let mut tw = boot_over(world, store.clone(), table);
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
    for who in reconnect {
        let _ = tw.connect(*who);
    }
    Rig {
        tw,
        oracle,
        store,
        typed,
        ledger,
        table,
        start,
    }
}

/// Fold every write and reboot with the stored `tacsim:` document `key` removed.
fn reboot_without(mut rig: Rig, key: &str, reconnect: &[ObjId]) -> Rig {
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    for data in rig.store.values_mut() {
        data.kv.retain(|(k, _)| k.as_str() != key);
    }
    reboot(rig, reconnect)
}

/// Everything due `secs` real seconds on.
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

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&cid(LANE_PACK), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn lane() -> RulesPack {
    load_pack(LANE_JSON).expect("the lane verifies")
}

fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = match lane().entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    };
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    tacsim::port::apply::room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let said = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(
        tw.world().get(who).and_then(|o| o.location),
        Some(room),
        "the teleport landed: {said}"
    );
}

/// A fresh request/commit token per landing, across the process.
fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x7c_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Keep a prepared bundle's typed post-images for a reboot.
fn keep(rig: &mut Rig, ops: Vec<mush_core::ModuleOp>) {
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

/// What [`land`] answers for a submission already in the harness's ledger (the HIT).
const HIT: &str = "HIT: answered from the idempotency record";

/// **A submission's ledger identity** (`mining.rs`'s): the fields the world task digests.
fn ledger_identity(submit: &AuthoritativeSubmit) -> String {
    json!({
        "actor": submit.actor,
        "action": submit.action,
        "subject": submit.subject,
        "targets": submit.targets,
        "parameters": submit.parameters,
        "key": submit.key,
    })
    .to_string()
}

/// A submission resolved against the world as it stands, not yet prepared.
struct Resolved {
    action: mush_core::DefinitionRef,
    resolved: mush_core::ResolvedActionInvocation,
    token: [u8; 16],
}

/// **The landing's first half**: the offer projected and resolved under a fresh request token.
fn resolve(rig: &Rig, submit: &AuthoritativeSubmit) -> Result<Resolved, String> {
    let token = next_token();
    let engine = rig.tw.engine();
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
    Ok(Resolved {
        action,
        resolved,
        token,
    })
}

/// **The landing's second half**: the prepare, the install, the typed post-images kept.
fn prepare(rig: &mut Rig, resolved: &Resolved) -> Result<Value, String> {
    let (prepared, output) = rig
        .tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &resolved.action,
            &resolved.resolved,
            CommitToken::from_bytes(resolved.token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let outcome = output.outcome().data().clone();
    let (ops, install) = prepared.into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .map_err(|error| format!("{error:?}"))?;
    keep(rig, ops);
    Ok(outcome)
}

/// **Drive one queued submission the way the world task does** (the ledger, then resolve and
/// prepare).
fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    let identity = ledger_identity(submit);
    if rig.ledger.contains(&identity) {
        return Err(HIT.to_owned());
    }
    let resolved = resolve(rig, submit)?;
    prepare(rig, &resolved)?;
    rig.ledger.insert(identity);
    Ok(())
}

/// Every queued submission of the port's lanes and industry's; asserts none of tacsim's or
/// industry's is GOD's (a tick runs as GOD and submits nothing itself).
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS || owner == "industry" {
            assert_ne!(submit.actor, GOD, "GOD never submits from a tick");
        }
        if owner == tacsim::NS || owner == "inventory" || owner == "market" || owner == "industry" {
            kept.push(*submit);
        }
    }
    kept
}

/// Land every submission; each must land or be the ledger's HIT.
fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        land(rig, submit)
            .or_else(|error| if error == HIT { Ok(()) } else { Err(error) })
            .unwrap_or_else(|error| {
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

/// One fire (6 s on), its submissions drained (not landed).
fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// One fire, everything it queued landed; the events.
fn fire_landing(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = fire(rig);
    land_all(rig, &queued);
    events
}

/// Fire and land the port tick until every port is stocked and listed.
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        let out = run(&mut rig.tw, GOD, "@tacsim/port status");
        let gated: Vec<&str> = out
            .lines()
            .filter(|line| {
                line.ends_with(']')
                    && !line.starts_with("sky: ")
                    && !line.starts_with("geology: ")
                    && !(line.contains(" site: ")
                        && (line.contains("; opening, ")
                            || line.contains("; opening: its ground is still being mapped")))
                    // Doc 20 cut SM.12b: the house's Pending span at Occator Rim (its seeds,
                    // licence and surveys) is the site's opening carried on — its length turns
                    // on the deposit table, as the opening's does — and is never stock.
                    && !(line.contains(" dispatch ") && line.ends_with("[pending]"))
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

/// A pilot on gateway account `account` (an alt shares one), standing at the Registry desk.
fn pilot_on(rig: &mut Rig, who: &str, account: &str) -> ObjId {
    let pilot = rig.tw.spawn_player(who);
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(pilot, AccountId::mint(account, 1).expect("an account id"));
    }
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    pilot
}

/// A chartered, licensed (two terms) pilot whose hull the yard has made a Dray-P.
fn prospector(rig: &mut Rig, who: &str, account: &str, name: &str) -> (ObjId, HullRow) {
    let pilot = pilot_on(rig, who, account);
    let out = run(&mut rig.tw, pilot, &format!("+coop/enrol {name}"));
    assert!(out.contains(&format!("now the {name}")), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row");
    let out = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert!(out.contains("as a Dray-P"), "{out}");
    support::licence::buy_licence(&mut rig.tw, pilot);
    // A second term, so the licence outlives a claim's first term.
    let out = run(&mut rig.tw, pilot, "+licence/buy go");
    assert!(out.contains("extends your licence"), "{out}");
    (pilot, row)
}

/// Seed `qty` of `good` into `vault`'s pool through `lot` (the inventory's own path).
fn seed_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    for step in 0..2 {
        let engine = rig.tw.engine_mut();
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
        keep(rig, ops);
    }
}

/// The hull's hold vault.
fn hold_vault(tw: &TestWorld, row: &HullRow) -> VaultId {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id
}

/// Board from C01.
fn board(rig: &mut Rig, pilot: ObjId) {
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
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

/// Sweep to the first real second whose IC instant is at or past `target`; the events and the
/// submissions queued (not landed).
fn advance_to(
    rig: &mut Rig,
    target: i64,
) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = ((ahead + rate - 1) / rate).max(1);
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// [`advance_to`], landing what it queued quietly.
fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = advance_to(rig, target);
    land_quietly(rig, &queued);
    events
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

/// File a passage with `line`, land its burn, launch it on the next fire; the ticket.
fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> Ticket {
    let (out, queued) = command(rig, pilot, line);
    assert!(out.contains("Clearance filed for "), "{line}: {out}");
    assert_eq!(queued.len(), 1, "one burn: {out}");
    let seq = match store(&rig.tw).voyage_state(hull) {
        tacsim::port::voyage::VoyageState::Present(voyage) => voyage
            .pending
            .as_ref()
            .map(|p| p.seq)
            .expect("a filed passage"),
        other => panic!("the sailing plan: {other:?}"),
    };
    land_all(rig, &queued);
    let t = ticket(&mut rig.tw, hull, seq).expect("the ticket");
    let (_, queued) = fire(rig);
    land_quietly(rig, &queued);
    t
}

/// Lease one rig with `+tv/lease go` at Occator's counter.
fn lease_one(rig: &mut Rig, pilot: ObjId) {
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, counter);
    let out = run(&mut rig.tw, pilot, "+tv/lease go");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );
}

/// The live lease records of `pilot`, by slot: their rigs.
fn rigs_of(tw: &TestWorld, pilot: ObjId) -> Vec<ObjId> {
    let mut records: Vec<(u8, ObjId)> = store(tw)
        .tv_leases_of(pilot)
        .filter(|record| record.live())
        .map(|record| (record.slot, record.tv))
        .collect();
    records.sort_unstable();
    records.into_iter().map(|(_, tv)| tv).collect()
}

/// Open `entry`, keeping the clerk's bundles.
fn open(rig: &mut Rig, entry: &str) {
    let mut kept = Vec::new();
    let _ = support::sites::open_site_keeping(&mut rig.tw, entry, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
}

/// **A Dray-P on a site's pad**, its pilot aboard, one rig leased; the site opened first and
/// its ground and home warmed.
fn on_pad_at(
    rig: &mut Rig,
    (who, account, name): (&str, &str, &str),
    (venue, entry): (&str, &str),
) -> (ObjId, HullRow, ObjId) {
    let (pilot, row) = prospector(rig, who, account, name);
    lease_one(rig, pilot);
    let vault = hold_vault(&rig.tw, &row);
    let lot = 9_100 + u64::from(row.hull) * 10;
    seed_pool(rig, vault, "propellant", 20, LotId(lot));
    seed_pool(rig, vault, "provisions", 20, LotId(lot + 1));
    board(rig, pilot);
    let hull = ObjId(row.hull);
    open(rig, entry);
    let t = file_and_launch(rig, pilot, hull, &format!("+ship/undock {venue}"));
    let _ = to_ic(rig, t.plan.depart_ic + t.plan.total_s);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == entry
        ),
        "the hull is settled on {entry}'s pad"
    );
    warm(rig, entry);
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), 1);
    (pilot, row, tvs[0])
}

/// [`on_pad_at`] on Ahuna Flank.
fn on_ahuna(rig: &mut Rig, who: &str, account: &str, name: &str) -> (ObjId, HullRow, ObjId) {
    on_pad_at(rig, (who, account, name), ("ahuna", AHUNA))
}

/// Fire until `entry`'s ground is cached and its home field built.
fn warm(rig: &mut Rig, entry: &str) {
    let (spec, location) = spec_of(&rig.tw, entry);
    // Doc 32 cut S.1: lane v17's seventh site is wanted too, and the tick warms one wanted
    // site a fire before it builds a home field, so the home can land a fire later.
    for _ in 0..16 {
        let cached =
            tacsim::surface::site_ground_cached(rig.tw.world(), &site_frame(&location), &spec)
                .ok()
                .flatten()
                .is_some();
        let homed = rig
            .tw
            .world()
            .module_state::<GeologyState>()
            .is_some_and(|state| {
                state
                    .homes()
                    .iter()
                    .any(|(site, ramp, _)| site.as_str() == entry && *ramp == RAMP_CELL)
            });
        if cached && homed {
            return;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("{entry}'s ground was not warmed");
}

/// A lane site's frame.
fn site_frame(location: &str) -> mush_atlas::window::WindowFrame {
    let region = format!("region.{location}");
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

/// A lane site's spec from its own lane entry and the world's installed body table.
fn spec_of(tw: &TestWorld, entry: &str) -> (SiteGeologySpec, String) {
    let pack = lane();
    let def = match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    };
    let location = match pack.entry(&def.location) {
        Some(PackEntry::Location(location)) => location.clone(),
        other => panic!("{} is a location: {other:?}", def.location),
    };
    let body = tacsim::surface::body_geology(tw.world(), location.body.as_ref().expect("a body"))
        .expect("the body's table is installed");
    let spec =
        tacsim::port::site::geology_spec(def.site.as_ref().expect("a site"), &location, &body)
            .expect("a spec");
    (spec, def.location.to_string())
}

/// A site's keyed ground, as a test reads it (a cache hit once warm).
fn ground_of(tw: &mut TestWorld, entry: &str) -> Arc<SiteGround> {
    let (spec, location) = spec_of(tw, entry);
    site_ground_sync(tw.world_mut(), &site_frame(&location), &spec).expect("the site builds")
}

/// The site's opening plan, from its keyed ground (the truth a test may read).
fn plan_of(tw: &mut TestWorld, entry: &str) -> Vec<OpeningRow> {
    let ground = ground_of(tw, entry);
    let site = match lane().entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.site.clone().expect("a site"),
        other => panic!("{entry}: {other:?}"),
    };
    let row = port_map(tw, entry).site.expect("a site row");
    opening_plan(&cid(entry), &site, &row, &ground.geology).expect("the plan reads")
}

/// A rig's typed record.
fn record_of(tw: &mut TestWorld, tv: ObjId) -> Option<RigRecord> {
    let h = handles(tw);
    typed(tw, &h.families.excursions, &RigKey(tv))
}

/// A principal's chart row for a site.
fn chart_of(tw: &mut TestWorld, who: ObjId, entry: &str) -> Option<ChartRecord> {
    let h = handles(tw);
    typed(
        tw,
        &h.families.charts,
        &ChartKey {
            principal: who,
            site: cid(entry),
        },
    )
}

/// **`line` as `who`, and exactly one start queued**, landed; the reply.
fn act_go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one start: {out}");
    assert_eq!(queued[0].actor, who, "the start is the operator's");
    land_all(rig, &queued);
    out
}

/// **Finish `tv`'s act**: to its due instant (the fire there queues the finish, landed), then one
/// more fire for the tell; what `who` heard over both.
fn finish(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight");
    let (events, queued) = advance_to(rig, due);
    let mut heard = said(&rig.tw, &events, who);
    land_all(rig, &queued);
    let (events, queued) = fire(rig);
    land_all(rig, &queued);
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, who));
    heard
}

/// Staff place `tv` `Direct` on `entry`'s domain at site cell `(x, y)`'s centre — and keep the
/// placement's writes for a reboot (doc 20 cut SM.11: `support::with_ctx` drops its outbox, so a
/// staff placement's recovery-ledger append never reached the store; harmless until a placement
/// sealed a ledger chunk, which the idle return's extra placements made happen here). The
/// placement itself is `support::place_keeping` (the lead's pre-commit round, item 5: shared with
/// `plans.rs`).
fn to_cell_on(rig: &mut Rig, entry: &str, tv: ObjId, cell: (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, entry).domain.expect("a domain"));
    let staff = support::place_keeping(&mut rig.tw, domain, tv, cell);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
}

/// Launch the pilot's rig 1 and finish it (it stands at the ramp).
fn launched(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    let _ = act_go(rig, pilot, "+tv/1 launch go");
    let _ = finish(rig, tv, pilot);
}

/// The soft stands of block `letter` of `entry` (a pocket's middle cell whose footing is not
/// rock), nearest the block's middle first, each with its pocket (from the keyed truth).
fn stands_in(tw: &mut TestWorld, entry: &str, letter: char) -> Vec<((i64, i64), (i64, i64))> {
    let ground = ground_of(tw, entry);
    let surface = ground
        .composite
        .surface(&mush_atlas::SurfaceRef::Ground)
        .expect("a ground surface");
    let (bx, by) = tacsim::surface::address::block_of_letter(letter).expect("a letter");
    let mut out = Vec::new();
    for py in by * 16..by * 16 + 16 {
        for px in bx * 16..bx * 16 + 16 {
            let stand = (px * 4 + 2, py * 4 + 2);
            if tacsim::surface::chart::footing_at(surface, stand)
                .is_ok_and(|footing| footing != tacsim::surface::reading::Footing::Rock)
            {
                out.push((stand, (px, py)));
            }
        }
    }
    let middle = (bx * 16 + 8, by * 16 + 8);
    out.sort_by_key(|(_, (px, py))| ((px - middle.0).abs().max((py - middle.1).abs()), *px, *py));
    out
}

/// A soft stand near the middle of block `letter`.
fn stand_in(tw: &mut TestWorld, entry: &str, letter: char) -> (i64, i64) {
    stands_in(tw, entry, letter)
        .first()
        .map(|(stand, _)| *stand)
        .unwrap_or_else(|| panic!("no soft stand in block {letter}"))
}

/// Put the pilot's rig on a soft stand of block `letter` (staff placement) and scan there; the
/// stand. The stand is the block's first soft one unless the pilot's last scan on the site was
/// made there (the same-cell rule, D4, would refuse it as nothing new): then the next one, so a
/// repeated reading always stamps fresh provenance.
fn scanned_in(rig: &mut Rig, entry: &str, pilot: ObjId, tv: ObjId, letter: char) -> (i64, i64) {
    let last = chart_of(&mut rig.tw, pilot, entry).and_then(|row| row.scan_stand);
    let stand = stands_in(&mut rig.tw, entry, letter)
        .into_iter()
        .map(|(stand, _)| stand)
        .find(|(x, y)| last != Some([*x, *y]))
        .unwrap_or_else(|| panic!("no fresh soft stand in block {letter}"));
    to_cell_on(rig, entry, tv, stand);
    let _ = act_go(rig, pilot, "+tv/1 scan go");
    let _ = finish(rig, tv, pilot);
    stand
}

/// **Put the rig back where it stood** after a long settle (doc 20 cut SM.11, lead ruling 6): at
/// this suite's rate a settle runs past a day, so the idle return brings a rig left on the ground
/// home; the rig is launched again (once its recover has landed) and staff put it back on
/// `stand`. A rig still out is only put back.
fn back_on(rig: &mut Rig, (entry, pilot, tv): (&str, ObjId, ObjId), stand: (i64, i64)) {
    if let Some(act) = record_of(&mut rig.tw, tv).and_then(|record| record.act) {
        let (_, queued) = advance_to(rig, act.due_ic);
        land_all(rig, &queued);
        let _ = fire_landing(rig);
    }
    let out = rig
        .tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .is_some_and(|state| matches!(state.parent_of(tv), mush_sim::SpatialParent::Direct { .. }));
    if !out {
        launched(rig, pilot, tv);
    }
    to_cell_on(rig, entry, tv, stand);
}

/// Sound (loud unless `quiet`) from where the rig stands; what the pilot heard.
fn sounded(rig: &mut Rig, pilot: ObjId, tv: ObjId, quiet: bool) -> String {
    let line = if quiet {
        "+tv/1 sound quiet go"
    } else {
        "+tv/1 sound go"
    };
    let _ = act_go(rig, pilot, line);
    finish(rig, tv, pilot)
}

// ---------------------------------------------------------------------------
// The claim readers
// ---------------------------------------------------------------------------

fn industry(tw: &TestWorld) -> IndustryFamilies {
    tw.world()
        .module_state::<IndustryHandles>()
        .expect("industry handles")
        .0
        .clone()
}

/// A block's public record.
fn block_record(tw: &mut TestWorld, entry: &str, letter: char) -> Option<BlockTenure> {
    let claims = handles(tw).families.claims;
    typed(
        tw,
        &claims,
        &BlockKey {
            site: cid(entry),
            letter,
        },
    )
}

/// The block's receipt-derived deposits that are open.
fn opened(tw: &mut TestWorld, entry: &str, letter: char) -> Vec<mush_industry::OreDepositId> {
    let families = industry(tw);
    OreFamily::ALL
        .into_iter()
        .filter_map(|family| {
            mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
                &cid(entry),
                letter,
                family,
            ))
            .ok()
        })
        .filter(|id| typed(tw, &families.deposits, id).is_some())
        .collect()
}

/// Every claim filed over a deposit.
fn claims_over(tw: &mut TestWorld, deposit: mush_industry::OreDepositId) -> Vec<ClaimRecord> {
    let families = industry(tw);
    let token = mush_industry::claim_subject_token(ClaimSubject::Deposit { deposit });
    let Some(index) = typed(tw, &families.claim_index, &token) else {
        return Vec::new();
    };
    index
        .claims()
        .iter()
        .filter_map(|id| typed(tw, &families.claims, id))
        .collect()
}

/// The block place's industry tenure.
fn tenure_of(tw: &mut TestWorld, entry: &str, letter: char) -> TenureRecord {
    let place = ObjId(
        port_map(tw, entry)
            .site
            .and_then(|row| row.blocks.get(&letter.to_string()).copied())
            .expect("the block place"),
    );
    let families = industry(tw);
    typed(tw, &families.tenure, &place).unwrap_or_else(|| TenureRecord::fresh(place))
}

/// A contract's state.
fn contract_state(tw: &TestWorld, id: u64) -> ContractState {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
        .contract(ContractId(id))
        .map(|contract| contract.state)
        .expect("the contract")
}

/// A pilot's available credit, minor units.
fn available(tw: &TestWorld, who: ObjId) -> i64 {
    tacsim::port::credit::own_account(tw.world(), who)
        .and_then(mush_economy::Account::available)
        .map_or(0, mush_economy::Amount::minor)
}

/// `+claim…` as `who`: the reply and the submissions it queued.
fn claim(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    command(rig, who, line)
}

/// **`+claim/<act> … go` as `who`, exactly one `industry_claim_tenure` queued**, landed; the reply.
fn claim_go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one tenure act: {out}");
    assert_eq!(queued[0].actor, who);
    assert_eq!(
        queued[0].action.key().id.as_str(),
        mush_industry::ACTION_CLAIM_TENURE
    );
    land_all(rig, &queued);
    out
}

/// Port fires until the site clerk's queue is quiet for two fires (bounded at 40), every
/// submission landed; what `who` heard over them. The house's own submissions (doc 20 cut SM.12b:
/// its dispatcher's drones at Occator Rim, whose work turns on the deposit table) are landed and
/// never break the quiet.
fn settle(rig: &mut Rig, who: &[ObjId]) -> String {
    let mut heard = String::new();
    let mut quiet = 0;
    for _ in 0..40 {
        let (events, queued) = fire(rig);
        for pilot in who {
            let text = said(&rig.tw, &events, *pilot);
            if !text.is_empty() {
                heard.push_str(&text);
                heard.push('\n');
            }
        }
        let house = queued
            .iter()
            .all(|submit| support::drones::is_house(&rig.tw, submit));
        land_all(rig, &queued);
        if house {
            quiet += 1;
            if quiet >= 2 {
                break;
            }
        } else {
            quiet = 0;
        }
    }
    heard
}

/// The economy's settled lines naming `contract` in the chronicle.
fn settled_lines(tw: &TestWorld, contract: u64) -> usize {
    let Some(state) = tw.world().module_state::<mush_canon::CanonState>() else {
        return 0;
    };
    state
        .recent()
        .iter()
        .filter(|event| {
            event.kind.as_str() == "economy.settled"
                && event.detail.get("contract").and_then(Value::as_u64) == Some(contract)
        })
        .count()
}

/// The `tacsim.claim.lapsed` lines for one block epoch.
fn lapse_lines(tw: &TestWorld, entry: &str, letter: char, epoch: u32) -> usize {
    let Some(state) = tw.world().module_state::<mush_canon::CanonState>() else {
        return 0;
    };
    mush_canon::records_for_source(
        state,
        &format!("tacsim.claim.lapse/{entry}/{letter}/{epoch}"),
        mush_canon::MAX_RECORDS_PER_SOURCE,
    )
    .iter()
    .filter(|event| event.kind.as_str() == cl::KIND_CLAIM_LAPSED)
    .count()
}

// ---------------------------------------------------------------------------
// C3 — the stake files the block and its fee reaches the chronicle
// ---------------------------------------------------------------------------

/// **C3 — a stake files the whole block and its fee reaches the chronicle** (§13's SM.10 row): on
/// the lane's own Ceres table, after a hand scan of block C, a bare `+claim/stake` previews and
/// queues nothing; `go` queues exactly one `industry_claim_tenure`, escrows the 4,000 cr fee in an
/// `Offered` contract and replies `STAKE_FILED`; landed, every opened deposit of C carries one
/// Registry claim held by the pilot at one term and the block record names the pilot; the next
/// port fire's clerk accepts the fee (`STAKE_RECORDED` told), the economy settles it, and exactly
/// one `economy.settled` chronicle line names it.
#[test]
fn sm10_a_stake_files_the_whole_block_and_its_fee_reaches_the_chronicle() {
    let mut rig = world_on(None);
    let (pilot, _, tv) = on_ahuna(&mut rig, "Pilot", "PilotAcct", "Salt Moth");
    launched(&mut rig, pilot, tv);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = scanned_in(&mut rig, AHUNA, pilot, tv, letter);
    let before = available(&rig.tw, pilot);
    let (preview, queued) = claim(&mut rig, pilot, "+claim/stake");
    assert!(queued.is_empty(), "a preview queues nothing: {preview}");
    assert!(
        preview.starts_with(&format!("Block {block}, your readings: ")),
        "{preview}"
    );
    assert!(
        preview.contains("2000.00cr now and 2000.00cr a term (4000.00cr today)."),
        "{preview}"
    );
    assert_eq!(
        available(&rig.tw, pilot),
        before,
        "the preview holds nothing"
    );
    let out = claim_go(&mut rig, pilot, "+claim/stake go");
    assert!(
        out.contains(&format!(
            "You stake block {block} at Ahuna Flank; 4000.00cr is held until the Registry \
             records it, and comes back within the hour if it does not."
        )),
        "{out}"
    );
    assert_eq!(available(&rig.tw, pilot), before - 400_000, "escrowed");
    let record = block_record(&mut rig.tw, AHUNA, letter).expect("the block record");
    let claim = record.claim.clone().expect("the claim");
    assert_eq!(claim.holder, mush_org::PrincipalId::Object(pilot));
    assert_eq!(claim.epoch, 0);
    let fee = claim.fee.expect("the fee contract");
    assert_eq!(contract_state(&rig.tw, fee), ContractState::Offered);
    let deposits = opened(&mut rig.tw, AHUNA, letter);
    assert!(!deposits.is_empty(), "the block holds ore");
    for deposit in &deposits {
        let claims = claims_over(&mut rig.tw, *deposit);
        assert_eq!(claims.len(), 1, "one claim a deposit");
        assert_eq!(claims[0].holder(), &mush_org::PrincipalId::Object(pilot));
        assert_eq!(claims[0].term().at, claim.term_end_ic, "one term");
    }
    let heard = settle(&mut rig, &[pilot]);
    assert!(
        heard.contains(&format!(
            "The {REGISTRY} records your claim on block {block} at Ahuna Flank until "
        )),
        "{heard}"
    );
    for _ in 0..30 {
        if matches!(contract_state(&rig.tw, fee), ContractState::Settled { .. }) {
            break;
        }
        sweep(&mut rig.tw, 60);
    }
    assert!(matches!(
        contract_state(&rig.tw, fee),
        ContractState::Settled { .. }
    ));
    sweep(&mut rig.tw, 120);
    assert_eq!(settled_lines(&rig.tw, fee), 1, "one economy.settled line");
}

// ---------------------------------------------------------------------------
// More of the rig: the clock, the money, the mining
// ---------------------------------------------------------------------------

/// The real instant every cross-world sequence starts at, seconds after the boot (the IC
/// instant is a pure function of it once the rate is set at the same instant in each world).
const ALIGN_SECS: i64 = 5_000;

/// Bring the world's real clock to `ALIGN_SECS` after its boot, everything due landed, so a
/// sequence that follows reads the same IC instants in every world.
fn align(rig: &mut Rig) {
    let target = rig.start + ALIGN_SECS;
    assert!(
        rig.tw.world().now() < target,
        "the setup ran past the alignment"
    );
    rig.tw.set_now(target);
    let _ = rig.tw.fire_due(target.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
}

/// The tools table's battery row.
fn battery() -> tacsim::definitions::tools::BatteryDef {
    *tacsim::definitions::tools::tools_table()
        .expect("the tools table")
        .values()
        .find_map(tacsim::definitions::tools::ToolDef::battery)
        .expect("a battery row")
}

/// Write `record` into the excursion family through a typed transaction (staff's hand).
fn put_record(rig: &mut Rig, record: RigRecord) {
    let family = handles(&rig.tw).families.excursions;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("an excursion transaction");
        let key = record.key();
        let exists = txn.get(&family, &key).expect("reads").is_some();
        if exists {
            txn.replace(&family, &key, record).expect("replaced");
        } else {
            txn.insert(&family, &key, record).expect("inserted");
        }
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
}

/// A full battery out of the bay at the world's IC now (staff's hand; the record kept).
fn recharge(rig: &mut Rig, tv: ObjId) {
    let battery = battery();
    let mut record = record_of(&mut rig.tw, tv).unwrap_or_else(|| RigRecord::fresh(tv, &battery));
    record.charge = tacsim::surface::Charge {
        value_cu: battery.capacity_cu,
        at_ic: ic_now(&rig.tw),
        in_bay: false,
    };
    put_record(rig, record);
}

/// The receipt-derived deposit ids of block `letter`, opened or not.
fn block_ids(entry: &str, letter: char) -> Vec<mush_industry::OreDepositId> {
    OreFamily::ALL
        .into_iter()
        .filter_map(|family| {
            mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
                &cid(entry),
                letter,
                family,
            ))
            .ok()
        })
        .collect()
}

/// Σ `drawn_t` over block `letter`'s cursors.
fn drawn_block(tw: &mut TestWorld, entry: &str, letter: char) -> u64 {
    let pockets = handles(tw).families.pockets;
    block_ids(entry, letter)
        .into_iter()
        .filter_map(|id| typed(tw, &pockets, &id))
        .filter_map(|cursor| cursor.drawn_total())
        .sum()
}

/// `+tv/1 mine go` from where the rig stands, every cycle landed at its due; `false` when the
/// start was refused (nothing queued).
fn mine_out(rig: &mut Rig, pilot: ObjId, tv: ObjId) -> bool {
    let (_, queued) = command(rig, pilot, "+tv/1 mine go");
    if queued.len() != 1 {
        return false;
    }
    land_all(rig, &queued);
    for _ in 0..16 {
        let Some(run) = record_of(&mut rig.tw, tv).and_then(|record| record.run) else {
            break;
        };
        let (_, queued) = advance_to(rig, run.due_ic);
        land_all(rig, &queued);
    }
    let _ = fire_landing(rig);
    true
}

/// `+tv/1 dump go`, finished.
fn dumped(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    let _ = act_go(rig, pilot, "+tv/1 dump go");
    let _ = finish(rig, tv, pilot);
}

/// **Work at least `tonnes` of block `letter`** by real cycles — sound quietly at a stand, mine
/// it out run by run, dump between hoppers, move to the next stand — never a cursor edit; the
/// battery recharged by staff's hand between runs. The tonnes worked.
fn work(rig: &mut Rig, entry: &str, pilot: ObjId, tv: ObjId, letter: char, tonnes: u64) -> u64 {
    let start = drawn_block(&mut rig.tw, entry, letter);
    for (stand, _) in stands_in(&mut rig.tw, entry, letter) {
        if drawn_block(&mut rig.tw, entry, letter) - start >= tonnes {
            break;
        }
        to_cell_on(rig, entry, tv, stand);
        recharge(rig, tv);
        let _ = sounded(rig, pilot, tv, true);
        for _ in 0..8 {
            if drawn_block(&mut rig.tw, entry, letter) - start >= tonnes {
                break;
            }
            recharge(rig, tv);
            let before = drawn_block(&mut rig.tw, entry, letter);
            if !mine_out(rig, pilot, tv) {
                break;
            }
            dumped(rig, pilot, tv);
            if drawn_block(&mut rig.tw, entry, letter) == before {
                break;
            }
        }
    }
    let worked = drawn_block(&mut rig.tw, entry, letter) - start;
    assert!(worked >= tonnes, "worked {worked} of {tonnes} t");
    worked
}

/// Stand the pilot's rig in block `letter`, scan, stake it (`go`), land, and let the clerk
/// record the fee; the block's claim.
fn stake(
    rig: &mut Rig,
    entry: &str,
    pilot: ObjId,
    tv: ObjId,
    letter: char,
) -> tacsim::surface::claims::BlockClaim {
    let stand = scanned_in(rig, entry, pilot, tv, letter);
    let out = claim_go(rig, pilot, "+claim/stake go");
    assert!(
        out.contains(&format!("You stake block {letter} at ")),
        "{out}"
    );
    let _ = settle(rig, &[pilot]);
    back_on(rig, (entry, pilot, tv), stand);
    let claim = block_record(&mut rig.tw, entry, letter)
        .and_then(|record| record.claim)
        .expect("the claim");
    assert!(matches!(
        contract_state(&rig.tw, claim.fee.expect("a fee")),
        ContractState::Active | ContractState::ReadyToSettle | ContractState::Settled { .. }
    ));
    claim
}

/// A pilot's credit account id.
fn credit_of(tw: &TestWorld, who: ObjId) -> u64 {
    tacsim::port::credit::own_account(tw.world(), who)
        .map(|account| account.id.0)
        .expect("a credit account")
}

/// The Registry's account balance, minor units.
fn registry_balance(tw: &TestWorld) -> i64 {
    let row = port_map(tw, "ceres_port").orgs[&cid("freeport_registry")].clone();
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
        .account(mush_economy::AccountId(row.account.expect("an account")))
        .map(|account| account.posted_balance.minor())
        .expect("the Registry's account")
}

/// The tenure acts among `queued` and their actors.
fn tenure_actors(queued: &[AuthoritativeSubmit]) -> Vec<ObjId> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == mush_industry::ACTION_CLAIM_TENURE)
        .map(|submit| submit.actor)
        .collect()
}

/// The site clerk Thing of `entry`.
fn clerk_of(tw: &TestWorld, entry: &str) -> ObjId {
    ObjId(
        port_map(tw, entry)
            .site
            .and_then(|row| row.clerk)
            .expect("a clerk"),
    )
}

/// A `site.map` query frame for `who` (`mining.rs`'s).
fn map_frame(rig: &mut Rig, who: ObjId, args: Value) -> Value {
    feed_frame(rig, who, "site.map", args)
}

/// A query frame of feed `name` for `who`, through the feed's own query (`quarry.rs`'s
/// `ground`).
fn feed_frame(rig: &mut Rig, who: ObjId, name: &str, args: Value) -> Value {
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    rig.tw.engine_mut().step(
        mush_core::Inbound::Query {
            conn: 7,
            actor: who,
            id: 1,
            name: SmolStr::new(name),
            args,
        },
        &mut out,
        &mut fx,
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            OutputEvent::Reply { event, .. } => match event.as_ref() {
                OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| panic!("a {name} frame"))
}

/// **A site's tenure line as `+chart` prints it** (`sites.rs`'s `under`): the line two after the
/// site's passage line `  {site}: …` (its survey line between).
fn tenure_under(chart: &str, site: &str) -> String {
    let lines: Vec<&str> = chart.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with(&format!("  {site}: ")))
        .unwrap_or_else(|| panic!("no line for {site}: {chart}"));
    lines.get(at + 2).copied().unwrap_or("").to_owned()
}

/// **A site's tenure, read every way a player reads it** (lead note 0a): `+chart`'s line and
/// the `sky.chart` feed's copy of it for `walker` (at the Freeport, so the site is a
/// destination), and `who`'s `body.ground` row's `tenure` word and `+chart/ground`'s line. The
/// four agree; the word.
fn tenure_read(rig: &mut Rig, walker: ObjId, who: ObjId, entry: &str, site: &str) -> String {
    let chart = run(&mut rig.tw, walker, "+chart");
    let line = tenure_under(&chart, site);
    let sky = feed_frame(rig, walker, "sky.chart", json!({}));
    let fed: Vec<String> = sky["chart"]["lines"]["sky"]
        .as_array()
        .expect("the sky lines")
        .iter()
        .filter_map(|line| line.as_str().map(str::to_owned))
        .collect();
    assert_eq!(tenure_under(&fed.join("\n"), site), line, "the feed's copy");
    let ground = feed_frame(rig, who, "body.ground", json!({}));
    let word = ground["ground"]["sites"]
        .as_array()
        .expect("the ground's sites")
        .iter()
        .find(|row| row["entry"] == json!(entry))
        .and_then(|row| row["tenure"].as_str())
        .unwrap_or_else(|| panic!("{entry}'s ground row: {ground}"))
        .to_owned();
    assert_eq!(
        line,
        tacsim::port::site::TENURE_LINE.replace("{parts}", &word),
        "+chart's line and the ground chart's word"
    );
    let listed = run(&mut rig.tw, who, "+chart/ground");
    assert!(listed.contains(&line), "+chart/ground: {listed}");
    word
}

/// A frame's rows as glyph strings, row 0 the south edge.
fn frame_rows(frame: &Value) -> Vec<String> {
    let width = usize::try_from(frame["width"].as_u64().expect("a width")).expect("fits");
    let height = usize::try_from(frame["height"].as_u64().expect("a height")).expect("fits");
    let rows = frame["rows"].as_array().expect("rows");
    mush_ansi::panel::window_rows(rows, width, height, 0, 0, width, height)
        .into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

/// The frame cells holding `#`.
fn claim_cells(frame: &Value) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (y, row) in frame_rows(frame).iter().enumerate() {
        for (x, glyph) in row.chars().enumerate() {
            if glyph == '#' {
                out.push((x, y));
            }
        }
    }
    out
}

/// The per-block opened-deposit counts of `entry` under the world's table (the plan's).
fn deposit_counts(tw: &mut TestWorld, entry: &str) -> BTreeMap<char, usize> {
    let mut counts: BTreeMap<char, usize> = tacsim::port::def::BLOCK_LETTERS
        .into_iter()
        .map(|letter| (letter, 0))
        .collect();
    for row in plan_of(tw, entry) {
        *counts.entry(row.letter).or_default() += 1;
    }
    counts
}

/// **The deposit-count rig** (reviewer F10): a keyed world on `table`, Ahuna Flank opened, and
/// every block's opened-deposit count asserted from the opening plan (4, 1 or 0) and from
/// industry's own deposits, so no test on it passes vacuously.
fn world_with_table(table: &'static str) -> Rig {
    let mut rig = world_on(Some(table));
    open(&mut rig, AHUNA);
    let want = if table == ALL_RICH {
        4
    } else if table == METALS_ONLY {
        1
    } else if table == ALL_BARREN {
        0
    } else {
        panic!("an unknown table")
    };
    let counts = deposit_counts(&mut rig.tw, AHUNA);
    assert!(
        counts.values().all(|n| *n == want),
        "every block opens {want}: {counts:?}"
    );
    for letter in tacsim::port::def::BLOCK_LETTERS {
        assert_eq!(
            opened(&mut rig.tw, AHUNA, letter).len(),
            want,
            "block {letter}"
        );
    }
    rig
}

/// The block of `entry` with the most opened deposits under the world's table (at least one).
fn ore_block(tw: &mut TestWorld, entry: &str) -> char {
    let mut best = ('A', 0_usize);
    for letter in tacsim::port::def::BLOCK_LETTERS {
        let n = opened(tw, entry, letter).len();
        if n > best.1 {
            best = (letter, n);
        }
    }
    assert!(best.1 > 0, "{entry} has a block with ore");
    best.0
}

/// The lines of `text` that are not the economy's own notices nor the submission's echo.
fn claim_lines(text: &str) -> String {
    text.lines()
        .filter(|line| {
            !line.starts_with("Contract #") && *line != "Queued." && !line.trim().is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Re-license `pilot` (a new term from anywhere).
fn relicense(rig: &mut Rig, pilot: ObjId) {
    let out = run(&mut rig.tw, pilot, "+licence/buy go");
    assert!(
        out.contains("licenses you to prospect") || out.contains("extends your licence"),
        "{out}"
    );
}

/// The IC instant `days` IC days on, every queued act landed quietly, then the clerk settled.
fn days_on(rig: &mut Rig, days: i64, who: &[ObjId]) -> String {
    let target = ic_now(&rig.tw) + days * DAY;
    let events = to_ic(rig, target);
    let mut heard = String::new();
    for pilot in who {
        heard.push_str(&said(&rig.tw, &events, *pilot));
        heard.push('\n');
    }
    heard.push_str(&settle(rig, who));
    heard
}

// ---------------------------------------------------------------------------
// C1 — evidence
// ---------------------------------------------------------------------------

/// **C1 — staking needs a hand-made reading of the block** (§9.3, lead ruling 2): with no
/// reading, with only another pilot's reading, and with a reading older than the block's last
/// opening, the stake answers `STAKE_NO_EVIDENCE` byte-identically and queues nothing; a single
/// blind core (a reading of its own) suffices. (`via: Plan`, which nothing writes yet, is the pure
/// rule's own unit test, `surface::tenure`.)
#[test]
fn sm10_staking_needs_a_hand_made_reading_of_the_block() {
    let mut rig = world_on(Some(ALL_RICH));
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let refused = cl::STAKE_NO_EVIDENCE.replace("{block}", "C");
    // 1. No reading at all.
    let stand = stand_in(&mut rig.tw, AHUNA, 'C');
    to_cell_on(&mut rig, AHUNA, ta, stand);
    let (first, queued) = claim(&mut rig, a, "+claim/stake");
    assert_eq!(first.trim(), refused);
    assert!(queued.is_empty());
    // 2. Another pilot's reading of the block.
    let _ = scanned_in(&mut rig, AHUNA, b, tb, 'C');
    let (second, queued) = claim(&mut rig, a, "+claim/stake go");
    assert_eq!(second, first, "another pilot's reading never counts");
    assert!(queued.is_empty(), "nothing is filed");
    // 3. A reading from before the block last opened: A scans, B stakes and releases.
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let _ = stake(&mut rig, AHUNA, b, tb, 'C');
    let out = claim_go(&mut rig, b, "+claim/release C go");
    assert!(out.contains("You release block C at Ahuna Flank."), "{out}");
    let _ = settle(&mut rig, &[a, b]);
    to_cell_on(&mut rig, AHUNA, ta, stand);
    let (third, queued) = claim(&mut rig, a, "+claim/stake");
    assert_eq!(third, first, "a reading older than the epoch never counts");
    assert!(queued.is_empty());
    // 4. One blind core of its own suffices.
    recharge(&mut rig, ta);
    assert!(mine_out(&mut rig, a, ta), "the blind core starts");
    let (preview, queued) = claim(&mut rig, a, "+claim/stake");
    assert!(queued.is_empty());
    // The clause lists the pre-epoch scans too (four at most, then "and n more"): the core is
    // what passes the rule, and the scanned stands are the block's first-listed fields.
    assert!(
        preview.starts_with("Block C, your readings: ")
            && (preview.contains(" cored") || preview.contains(" more;")),
        "{preview}"
    );
    assert_ne!(preview, first, "the core is evidence");
}

// ---------------------------------------------------------------------------
// C2 — the same whatever the block holds
// ---------------------------------------------------------------------------

/// What one C2 world printed and charged.
#[derive(Debug, PartialEq, Eq)]
struct StakeSeen {
    ic: i64,
    preview: String,
    filed: String,
    recorded: String,
    amounts: (i64, Vec<i64>),
    cells: Vec<(usize, usize)>,
    cells_other: Vec<(usize, usize)>,
    claims: Value,
}

/// C2's sequence on `table`: two other pilots sound block C before Ada — Bryn loudly from one
/// stand and quietly from a second, Cato loudly from a third (two principals' rows, so the
/// heard count's paged scan of the site's chart rows is what counts them) — then Ada scans it,
/// previews (nothing filed), stakes and is recorded.
fn c2_world(table: &'static str) -> StakeSeen {
    let mut rig = world_with_table(table);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    let (c, _, tc) = on_ahuna(&mut rig, "Cato", "CatoAcct", "Tern");
    // Doc 20 cut SM.11: aligned before the launches, so the jump to the aligned instant never
    // leaves a rig a day idle on the ground (the idle return would bring it home).
    align(&mut rig);
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    launched(&mut rig, c, tc);
    let ic = ic_now(&rig.tw);
    let stands = stands_in(&mut rig.tw, AHUNA, 'C');
    for (who, tv, i, quiet) in [(b, tb, 0, false), (b, tb, 1, true), (c, tc, 2, false)] {
        to_cell_on(&mut rig, AHUNA, tv, stands[i].0);
        let _ = sounded(&mut rig, who, tv, quiet);
    }
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let contracts = |tw: &TestWorld| {
        tw.world()
            .module_state::<EconomyStore>()
            .expect("economy")
            .snapshot()
            .contracts()
            .count()
    };
    let before = contracts(&rig.tw);
    let (preview, queued) = claim(&mut rig, a, "+claim/stake");
    assert!(queued.is_empty(), "a preview submits nothing");
    assert_eq!(contracts(&rig.tw), before, "a preview offers no contract");
    assert_eq!(
        cl::claim_order_for_tests(rig.tw.world(), a),
        None,
        "nor an order"
    );
    assert!(
        preview.contains("soundings heard on this block since it last opened, before yours: 2;"),
        "{preview}"
    );
    let filed = claim_lines(&claim_go(&mut rig, a, "+claim/stake go"));
    let recorded = claim_lines(&settle(&mut rig, &[a]));
    let fee = block_record(&mut rig.tw, AHUNA, 'C')
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    let amounts = {
        let economy = rig
            .tw
            .world()
            .module_state::<EconomyStore>()
            .expect("economy");
        let contract = economy
            .snapshot()
            .contract(ContractId(fee))
            .expect("the contract")
            .clone();
        (
            contract
                .consent
                .terms
                .consideration
                .iter()
                .map(|row| row.amount.minor())
                .sum(),
            contract
                .consent
                .terms
                .outcomes
                .iter()
                .flat_map(|outcome| outcome.payouts.iter())
                .map(|payout| match payout.share {
                    mush_economy::Share::Exact(amount) => amount.minor(),
                    _ => -1,
                })
                .collect(),
        )
    };
    let frame = map_frame(&mut rig, a, json!({ "view": "site" }));
    let other = map_frame(&mut rig, b, json!({ "view": "site" }));
    StakeSeen {
        ic,
        preview,
        filed,
        recorded,
        amounts,
        cells: claim_cells(&frame),
        cells_other: claim_cells(&other),
        claims: frame["site"]["claims"].clone(),
    }
}

/// **C2 — a stake reads the same whatever the block holds** (lead ruling 1; SM.6's D14): the same
/// pilots do the same acts in three worlds whose block C holds four, one and no opened deposits;
/// the preview (heard count 2: Bryn's and Cato's loud soundings, Bryn's quiet one unheard), the
/// filed line, the
/// recorded line, the fee contract's amounts and the map's `#` cells are byte-identical, and a
/// bare stake offers no contract, writes no order and submits nothing.
#[test]
fn sm10_a_stake_reads_the_same_whatever_the_block_holds() {
    let rich = c2_world(ALL_RICH);
    let metals = c2_world(METALS_ONLY);
    let barren = c2_world(ALL_BARREN);
    println!("C2: {rich:#?}");
    assert_eq!(rich.amounts, (400_000, vec![400_000]));
    assert!(!rich.cells.is_empty(), "the block's edge is drawn");
    assert_eq!(
        rich.cells, rich.cells_other,
        "the same grid for every viewer"
    );
    assert_eq!(rich, metals, "four deposits and one read alike");
    assert_eq!(metals, barren, "one deposit and none read alike");
}

// ---------------------------------------------------------------------------
// C4 — a second stake; the paper bars nothing
// ---------------------------------------------------------------------------

/// **C4 — a second stake refuses and a paper bars nothing** (§9.2, §9.6): Bryn's stake on Ada's
/// block answers "That block is under claim."; at Occator Rim, once the clerk has seeded the
/// Syndicate's paper, Cato stakes a papered outer block and mines it.
#[test]
fn sm10_a_second_stake_refuses_and_paper_bars_nothing() {
    let mut rig = world_on(Some(ALL_RICH));
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let _ = stake(&mut rig, AHUNA, a, ta, letter);
    let _ = scanned_in(&mut rig, AHUNA, b, tb, letter);
    let (out, queued) = claim(&mut rig, b, "+claim/stake go");
    assert_eq!(out.trim(), "That block is under claim.");
    assert!(queued.is_empty());
    // Occator Rim: the paper is seeded, and bars nothing.
    let (c, _, tc) = on_pad_at(&mut rig, ("Cato", "CatoAcct", "Tern"), ("occator", OCCATOR));
    let _ = settle(&mut rig, &[c]);
    let record = block_record(&mut rig.tw, OCCATOR, 'B').expect("B's record");
    assert_eq!(
        record.paper.as_ref().map(|paper| paper.state),
        Some(PaperState::Contested)
    );
    launched(&mut rig, c, tc);
    let claimed = stake(&mut rig, OCCATOR, c, tc, 'B');
    assert_eq!(claimed.holder, mush_org::PrincipalId::Object(c));
    recharge(&mut rig, tc);
    let (preview, _) = command(&mut rig, c, "+tv/1 mine");
    assert!(!preview.contains("That block is under claim."), "{preview}");
    assert!(
        mine_out(&mut rig, c, tc),
        "the papered block's holder mines it"
    );
}

// ---------------------------------------------------------------------------
// C5 — a self-issued claim
// ---------------------------------------------------------------------------

/// **C5 — a self-issued claim authorises nothing at a Registry site** (§9.2, D-20-12): staff
/// file a claim over an Ahuna deposit issued by a player org of Pip's own; Sol, a stranger,
/// stakes the block under the Registry and mines it, and Pip's own mine is refused "That block
/// is under claim.".
#[test]
fn sm10_a_self_issued_claim_authorises_nothing_at_a_registry_site() {
    let mut rig = world_on(Some(ALL_RICH));
    let (p, _, tp) = on_ahuna(&mut rig, "Pip", "PipAcct", "Salt Moth");
    let (s, _, ts) = on_ahuna(&mut rig, "Sol", "SolAcct", "Grey Heron");
    launched(&mut rig, p, tp);
    launched(&mut rig, s, ts);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let out = run(&mut rig.tw, GOD, "@org/create Pip Holdings");
    let org: u64 = out
        .split("Created org #")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("{out}"));
    for deposit in opened(&mut rig.tw, AHUNA, letter) {
        let _ = run(
            &mut rig.tw,
            GOD,
            &format!(
                "@claim/file deposit:{deposit}=org:{org}/obj:{}/exclusive/1/100000000/transferable",
                p.0
            ),
        );
        // The staff command's own submission (GOD's, from a command, not a tick).
        let queued: Vec<AuthoritativeSubmit> =
            std::iter::from_fn(|| rig.tw.engine_mut().take_authoritative_submit())
                .map(|submit| *submit)
                .collect();
        land_all(&mut rig, &queued);
    }
    let deposit = opened(&mut rig.tw, AHUNA, letter)[0];
    assert_eq!(
        claims_over(&mut rig.tw, deposit).len(),
        1,
        "the self-issued claim"
    );
    let claimed = stake(&mut rig, AHUNA, s, ts, letter);
    assert_eq!(
        claimed.holder,
        mush_org::PrincipalId::Object(s),
        "it bars nobody"
    );
    recharge(&mut rig, ts);
    assert!(mine_out(&mut rig, s, ts), "the Registry's holder mines");
    let cell = stand_in(&mut rig.tw, AHUNA, letter);
    to_cell_on(&mut rig, AHUNA, tp, cell);
    recharge(&mut rig, tp);
    let (out, queued) = command(&mut rig, p, "+tv/1 mine go");
    assert!(out.contains("That block is under claim."), "{out}");
    assert!(queued.is_empty(), "it authorises nothing");
}

// ---------------------------------------------------------------------------
// C6, C7, C8 — renewal floor, the clerk's lapse, release
// ---------------------------------------------------------------------------

/// **C6 — a claim worked one cycle is not renewed and reopens** (§9.2): `RENEW_SHORT`; the clerk
/// lapses it stamped at the term's end; Sol takes the records over at once (the same claim ids,
/// the index unchanged); Ada, the lapsed holder, is refused `STAKE_COOLING` until the cooling
/// end and stakes once Sol has given it up and the term has run out.
#[test]
fn sm10_a_claim_worked_one_cycle_is_not_renewed_and_reopens() {
    let mut rig = world_on(Some(ALL_RICH));
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (s, _, ts) = on_ahuna(&mut rig, "Sol", "SolAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, s, ts);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let held = stake(&mut rig, AHUNA, a, ta, letter);
    recharge(&mut rig, ta);
    assert!(mine_out(&mut rig, a, ta), "one cycle");
    let worked = drawn_block(&mut rig.tw, AHUNA, letter);
    let (out, queued) = claim_cmd(&mut rig, a, &format!("+claim/renew {letter}"));
    assert_eq!(
        out.trim(),
        cl::RENEW_SHORT
            .replace("{block}", &block)
            .replace("{site}", "Ahuna Flank")
            .replace("{worked}", &worked.to_string())
            .replace("{floor}", "50")
            .replace(
                "{until}",
                &tacsim::port::licence::ic_stamp(held.term_end_ic)
            )
    );
    assert!(queued.is_empty());
    let ids_before: Vec<_> = opened(&mut rig.tw, AHUNA, letter)
        .into_iter()
        .flat_map(|deposit| claims_over(&mut rig.tw, deposit))
        .map(|claim| claim.id())
        .collect();
    // The term ends; the clerk lapses it, stamped at the term's end.
    let _ = to_ic(&mut rig, held.term_end_ic + 60);
    let _ = settle(&mut rig, &[a, s]);
    let record = block_record(&mut rig.tw, AHUNA, letter).expect("the record");
    assert_eq!(
        record.claim.as_ref().and_then(|c| c.lapsed_ic),
        Some(held.term_end_ic)
    );
    for deposit in opened(&mut rig.tw, AHUNA, letter) {
        for each in claims_over(&mut rig.tw, deposit) {
            assert_eq!(each.lapsed_at().map(|at| at.at), Some(held.term_end_ic));
        }
    }
    relicense(&mut rig, a);
    relicense(&mut rig, s);
    // The lapsed holder cools.
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let cooling_end = held.term_end_ic + tacsim::surface::tenure::COOLING_TERM_IC_SECS;
    let (out, queued) = claim(&mut rig, a, "+claim/stake go");
    assert_eq!(
        out.trim(),
        cl::STAKE_COOLING
            .replace("{block}", &block)
            .replace("{until}", &tacsim::port::licence::ic_stamp(cooling_end))
    );
    assert!(queued.is_empty());
    // A stranger takes the records over at once.
    let taken = stake(&mut rig, AHUNA, s, ts, letter);
    assert_eq!(taken.holder, mush_org::PrincipalId::Object(s));
    assert_eq!(taken.opened_ic, Some(held.term_end_ic));
    let ids_after: Vec<_> = opened(&mut rig.tw, AHUNA, letter)
        .into_iter()
        .flat_map(|deposit| claims_over(&mut rig.tw, deposit))
        .map(|claim| claim.id())
        .collect();
    assert_eq!(ids_before, ids_after, "taken over, never filed again");
    for deposit in opened(&mut rig.tw, AHUNA, letter) {
        assert_eq!(claims_over(&mut rig.tw, deposit).len(), 1);
    }
    assert_eq!(tenure_of(&mut rig.tw, AHUNA, letter).epoch(), 2);
    // Sol gives it up; past Ada's cooling end she stakes it again.
    let _ = claim_go(&mut rig, s, &format!("+claim/release {letter} go"));
    let _ = settle(&mut rig, &[a, s]);
    let _ = to_ic(&mut rig, cooling_end + 60);
    relicense(&mut rig, a);
    let _ = settle(&mut rig, &[a]);
    let again = stake(&mut rig, AHUNA, a, ta, letter);
    assert_eq!(again.holder, mush_org::PrincipalId::Object(a));
}

/// `+claim…` as `who` (alias of [`claim`] for lines built at runtime).
fn claim_cmd(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    command(rig, who, line)
}

/// **C7 — a lapse is the clerk's, with its canon line and its epoch** (§9.2, lead ruling 5):
/// past the term, before the clerk's act lands, a stranger's mine is allowed (R20) and
/// `+registry` reads "lapsing"; landed: `lapsed_at`, epoch + 1, one cooling row, exactly one
/// `tacsim.claim.lapsed` canon line across further fires and a reboot, and `LAPSED_TOLD`; the
/// applier and the boot pass submit nothing.
#[test]
fn sm10_a_lapse_is_the_clerks_with_its_canon_line_and_epoch() {
    let mut rig = world_on(Some(ALL_RICH));
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (s, _, ts) = on_ahuna(&mut rig, "Sol", "SolAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, s, ts);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let claim = stake(&mut rig, AHUNA, a, ta, letter);
    let clerk = clerk_of(&rig.tw, AHUNA);
    // Past the term: the clerk's lapse is queued, not yet landed.
    let (_, queued) = advance_to(&mut rig, claim.term_end_ic + 60);
    let lapses: Vec<&AuthoritativeSubmit> = queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == mush_industry::ACTION_CLAIM_TENURE)
        .collect();
    assert_eq!(lapses.len(), 1, "one lapse queued");
    assert_eq!(lapses[0].actor, clerk, "the clerk's");
    assert_eq!(
        lapses[0].key.as_str(),
        format!(
            "tacsim.claim.lapse/{AHUNA}/{}/0",
            letter.to_ascii_lowercase()
        )
    );
    let registry = run(&mut rig.tw, s, "+registry ahuna flank");
    assert!(
        registry.contains(&format!("  Block {block}: Ada; term ended "))
            && registry.contains(", lapsing."),
        "{registry}"
    );
    relicense(&mut rig, s);
    // The clerk's lapse is the only one (the fixer's round, conformance F4): the holder's release
    // and a rival's contest of a past-term claim each answer `STAKE_LAPSING` and queue nothing.
    let lapsing = cl::STAKE_LAPSING.replace("{block}", &block);
    for (who, line) in [
        (a, format!("+claim/release {letter} go")),
        (s, format!("+claim/contest {letter} go")),
    ] {
        let (out, none) = claim_cmd(&mut rig, who, &line);
        assert_eq!(out.trim(), lapsing, "{line}");
        assert!(none.is_empty(), "{line}");
    }
    let cell = stand_in(&mut rig.tw, AHUNA, letter);
    to_cell_on(&mut rig, AHUNA, ts, cell);
    recharge(&mut rig, ts);
    // A past-term claim reads as the authored word on the rig's ground line (SM.7's R20) ...
    let status = run(&mut rig.tw, s, "+tv");
    assert!(
        status.contains(&format!("Block {block} (open)")),
        "{status}"
    );
    // ... and a stranger's mine is allowed: the start is queued (and not landed before the
    // lapse — it is the gate's answer that is pinned).
    let (out, started) = command(&mut rig, s, "+tv/1 mine go");
    assert_eq!(started.len(), 1, "R20: the stranger's mine starts: {out}");
    assert_eq!(started[0].actor, s);
    assert!(!out.contains("That block is under claim."), "R20: {out}");
    land_all(&mut rig, &queued);
    let heard = settle(&mut rig, &[a]);
    let cooling_end = claim.term_end_ic + tacsim::surface::tenure::COOLING_TERM_IC_SECS;
    assert!(
        heard.contains(
            &cl::LAPSED_TOLD
                .replace("{block}", &block)
                .replace("{site}", "Ahuna Flank")
                .replace("{until}", &tacsim::port::licence::ic_stamp(cooling_end))
        ),
        "{heard}"
    );
    let tenure = tenure_of(&mut rig.tw, AHUNA, letter);
    assert_eq!(tenure.epoch(), 1);
    assert_eq!(tenure.cooling().len(), 1);
    assert_eq!(tenure.cooling()[0].holder, mush_org::PrincipalId::Object(a));
    assert_eq!(lapse_lines(&rig.tw, AHUNA, letter, 0), 1);
    for _ in 0..5 {
        let _ = fire_landing(&mut rig);
    }
    let rig_store_key = format!("tacsim:claimlog:{AHUNA}");
    let mut rig = reboot(rig, &[a, s]);
    let queued = drain(&mut rig.tw);
    assert!(
        tenure_actors(&queued).is_empty(),
        "the boot pass submits nothing"
    );
    let _ = settle(&mut rig, &[a]);
    assert_eq!(
        lapse_lines(&rig.tw, AHUNA, letter, 0),
        1,
        "once across a reboot"
    );
    // With the claim log gone, the chronicle's own guard holds it to one line.
    let mut rig = reboot_without(rig, &rig_store_key, &[a, s]);
    let heard = settle(&mut rig, &[a]);
    assert_eq!(
        lapse_lines(&rig.tw, AHUNA, letter, 0),
        1,
        "the chronicle guards it"
    );
    assert!(
        !heard.contains(&format!(
            "Your claim on block {block} at Ahuna Flank has lapsed;"
        )),
        "and the tell with it (conformance F9): {heard}"
    );
}

/// **C8 — release is a lapse by choice** (OC6): `RELEASE_PREVIEW`, then `RELEASE_FILED`; the
/// record lapsed now, the epoch + 1, the cooling row, one canon line and `LAPSED_TOLD`; before
/// the rig leaves the bay a stake answers `STAKE_IN_BAY`.
#[test]
fn sm10_release_is_a_lapse_by_choice() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    // A rig still in the bay stakes nothing (`STAKE_IN_BAY`).
    let (out, queued) = claim(&mut rig, a, "+claim/stake go");
    assert_eq!(out.trim(), cl::STAKE_IN_BAY.replace("{n}", "1"));
    assert!(queued.is_empty());
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = stake(&mut rig, AHUNA, a, ta, letter);
    let (preview, queued) = claim(&mut rig, a, "+claim/release");
    assert!(queued.is_empty());
    let until = tacsim::port::licence::ic_stamp(
        ic_now(&rig.tw) + tacsim::surface::tenure::COOLING_TERM_IC_SECS,
    );
    assert_eq!(
        preview.trim(),
        cl::RELEASE_PREVIEW
            .replace("{block}", &block)
            .replace("{site}", "Ahuna Flank")
            .replace("{until}", &until)
    );
    let at = ic_now(&rig.tw);
    let out = claim_go(&mut rig, a, "+claim/release go");
    assert!(
        out.contains(&format!("You release block {block} at Ahuna Flank.")),
        "{out}"
    );
    let record = block_record(&mut rig.tw, AHUNA, letter).expect("the record");
    let lapsed = record
        .claim
        .and_then(|claim| claim.lapsed_ic)
        .expect("lapsed");
    assert!(lapsed >= at, "lapsed now");
    let heard = settle(&mut rig, &[a]);
    assert!(
        heard.contains("has lapsed; the Registry takes no stake on it"),
        "{heard}"
    );
    let tenure = tenure_of(&mut rig.tw, AHUNA, letter);
    assert_eq!(tenure.epoch(), 1);
    assert_eq!(tenure.cooling().len(), 1);
    assert_eq!(lapse_lines(&rig.tw, AHUNA, letter, 0), 1);
}

// ---------------------------------------------------------------------------
// C9, C10 — the cooling term's reach and the cap
// ---------------------------------------------------------------------------

/// Map `who` to the gateway account `account` in the oracle (the host's account identity).
fn remap(rig: &Rig, who: ObjId, account: &str) {
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(who, AccountId::mint(account, 1).expect("an account id"));
    }
}

/// **C9 — the cooling term binds the holder, its controller and its account** (lead ruling 5,
/// OC15, reviewer F9): after Ada's release, Ada and Cass (who controls Ada) are refused
/// `STAKE_COOLING`; then Ada's character is mapped onto Alt's gateway account — Alt is then an
/// alt of the lapsed holder's account — and Alt is refused `STAKE_COOLING` too; Sol, a stranger
/// on another account, is not. (The licence is one holder an account, so a second character on
/// an account never reaches a stake of its own: the alt is made by moving the holder onto the
/// licensed alt's account, the host's account identity being what the rule reads.)
#[test]
fn sm10_the_cooling_term_binds_the_holder_its_controller_and_its_account() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (alt, _, talt) = on_ahuna(&mut rig, "Alt", "AltAcct", "Grey Heron");
    let (cass, _, tcass) = on_ahuna(&mut rig, "Cass", "CassAcct", "Tern");
    let (s, _, ts) = on_ahuna(&mut rig, "Sol", "SolAcct", "Kite");
    for (who, tv) in [(a, ta), (alt, talt), (cass, tcass), (s, ts)] {
        launched(&mut rig, who, tv);
    }
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = stake(&mut rig, AHUNA, a, ta, letter);
    let _ = claim_go(&mut rig, a, "+claim/release go");
    let _ = settle(&mut rig, &[a]);
    // Cass controls Ada (her owner).
    if let Some(obj) = rig.tw.world_mut().get_mut(a) {
        obj.owner = cass;
    }
    assert!(mush_economy::acts_for(
        rig.tw.world(),
        cass,
        &mush_org::PrincipalId::Object(a)
    ));
    let tenure = tenure_of(&mut rig.tw, AHUNA, letter);
    let until = tenure.cooling()[0].until.at;
    let cooling = cl::STAKE_COOLING
        .replace("{block}", &block)
        .replace("{until}", &tacsim::port::licence::ic_stamp(until));
    for (who, tv) in [(a, ta), (cass, tcass)] {
        let _ = scanned_in(&mut rig, AHUNA, who, tv, letter);
        let (out, queued) = claim(&mut rig, who, "+claim/stake go");
        assert_eq!(out.trim(), cooling, "#{} cools", who.0);
        assert!(queued.is_empty());
    }
    // Ada onto Alt's account: Alt is an alt of the lapsed holder's account.
    let _ = scanned_in(&mut rig, AHUNA, alt, talt, letter);
    let (out, _) = claim(&mut rig, alt, "+claim/stake");
    assert!(
        out.starts_with(&format!("Block {block}, your readings: ")),
        "{out}"
    );
    remap(&rig, a, "AltAcct");
    let (out, queued) = claim(&mut rig, alt, "+claim/stake go");
    assert_eq!(out.trim(), cooling, "the account cools");
    assert!(queued.is_empty());
    let _ = scanned_in(&mut rig, AHUNA, s, ts, letter);
    let (out, _) = claim(&mut rig, s, "+claim/stake");
    assert!(
        out.starts_with(&format!("Block {block}, your readings: ")),
        "{out}"
    );
}

/// **C10 — two live claims an account** (§9.2, reviewer F7): Ada holds two and a third refuses
/// `CLAIM_CAP`; with one live claim and her own stake order still `Offered` (filed, not landed),
/// another `go` refuses `CLAIM_CAP`; and when the slot has lost that order (a RAM eviction), so
/// two of her stakes are offered in one step, exactly one lands — the other's stage re-counts
/// the account inside its own bundle and refuses — and its escrow refunds at the deadline. (The
/// licence is one holder an account, so the account's claims are one licensed character's: the
/// account count is the per-account licence's twin.)
#[test]
fn sm10_two_live_claims_an_account() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let cap = cl::CLAIM_CAP.replace("{max}", "2");
    let _ = stake(&mut rig, AHUNA, a, ta, 'A');
    let _ = stake(&mut rig, AHUNA, a, ta, 'B');
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let (out, queued) = claim(&mut rig, a, "+claim/stake go");
    assert_eq!(out.trim(), cap);
    assert!(queued.is_empty());
    // One live claim, and a stake order Offered: another go refuses. (E is read first, so E's
    // `go` follows D's at once: an order older than `CLAIM_ORDER_TTL_IC_SECS` can never be
    // offered and no longer counts, and a scan takes IC time.)
    let _ = claim_go(&mut rig, a, "+claim/release B go");
    let _ = settle(&mut rig, &[a]);
    let e_cell = scanned_in(&mut rig, AHUNA, a, ta, 'E');
    let (out, d_queued) = {
        let _ = scanned_in(&mut rig, AHUNA, a, ta, 'D');
        claim(&mut rig, a, "+claim/stake go")
    };
    assert!(out.contains("You stake block D"), "{out}");
    // D's offer resolves while the rig still stands on D (the seam re-plans the order against
    // the rig's place, C22); it lands later, in the race.
    let d_resolved = resolve(&rig, &d_queued[0]).expect("D's offer");
    to_cell_on(&mut rig, AHUNA, ta, e_cell);
    let (out, queued) = claim(&mut rig, a, "+claim/stake go");
    assert_eq!(out.trim(), cap, "a pending order counts");
    assert!(queued.is_empty());
    // The race: D's offer resolved, its order evicted, E's offered too.
    cl::forget_claim_order_for_tests(rig.tw.world_mut(), a);
    // Funds for E's escrow (three fees paid or held already): the treasury pays Ada one fee.
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let credit = credit_of(&rig.tw, a);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay #{treasury}=#{credit} 400000"),
    );
    assert!(out.contains("Paid: transaction #"), "{out}");
    let before = available(&rig.tw, a);
    let (out, e_queued) = claim(&mut rig, a, "+claim/stake go");
    assert!(out.contains("You stake block E"), "{out}");
    let e_resolved = resolve(&rig, &e_queued[0]).expect("E's offer");
    prepare(&mut rig, &d_resolved).expect("D's stake lands");
    let refused = prepare(&mut rig, &e_resolved).expect_err("E's stage refuses");
    assert!(refused.contains("the most one account may"), "{refused}");
    assert!(
        block_record(&mut rig.tw, AHUNA, 'E').is_none_or(|record| record.claim.is_none()),
        "nothing written"
    );
    // E's escrow comes back at the fee's deadline.
    for _ in 0..80 {
        sweep(&mut rig.tw, 60);
    }
    assert_eq!(available(&rig.tw, a), before, "refunded");
}

// ---------------------------------------------------------------------------
// C11, C11b, C12 — the contest
// ---------------------------------------------------------------------------

/// **C11 — a heard sounding inside the epoch takes the claim** (§9.4, lead ruling 8): Bryn
/// sounds the block loudly, then Ada reads it and stakes it; Bryn's `+claim/contest` previews,
/// files and lands as ONE bundle (epoch + 1, holder Bryn, discovery the sounding's stamp); the
/// clerk accepts Bryn's 4,000 for the Registry (`CONTEST_RECORDED`, and `CONTEST_LOST` to Ada);
/// Ada's 2,000 term fee is refunded once, a reboot posts nothing twice, and the clerk still
/// lapses and chronicles afterwards (reviewer F6).
#[test]
fn sm10_a_heard_sounding_inside_the_epoch_takes_the_claim() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let cell = stand_in(&mut rig.tw, AHUNA, letter);
    to_cell_on(&mut rig, AHUNA, tb, cell);
    let heard_at = ic_now(&rig.tw);
    let _ = sounded(&mut rig, b, tb, false);
    let sounding = chart_of(&mut rig.tw, b, AHUNA)
        .and_then(|row| row.blocks.get(&block).cloned())
        .and_then(|cells| {
            cells
                .provenance
                .first()
                .map(tacsim::surface::ChartProvenance::at)
        })
        .expect("Bryn's sounding stamp");
    assert!(sounding >= heard_at);
    let held = stake(&mut rig, AHUNA, a, ta, letter);
    assert!(held.discovered_ic.is_some_and(|at| at > sounding));
    let a_fee = held.fee.expect("Ada's fee");
    for _ in 0..30 {
        if matches!(
            contract_state(&rig.tw, a_fee),
            ContractState::Settled { .. }
        ) {
            break;
        }
        sweep(&mut rig.tw, 60);
    }
    assert!(matches!(
        contract_state(&rig.tw, a_fee),
        ContractState::Settled { .. }
    ));
    let ada_before = available(&rig.tw, a);
    let (preview, queued) = claim(&mut rig, b, &format!("+claim/contest {letter}"));
    assert!(queued.is_empty());
    assert!(
        preview.starts_with(&format!(
            "Block {block} at Ahuna Flank: the site heard your sounding at {}, before Ada's claim rests (",
            tacsim::port::licence::ic_stamp(sounding)
        )) && preview.contains("4000.00cr takes the claim to"),
        "{preview}"
    );
    let epoch_before = tenure_of(&mut rig.tw, AHUNA, letter).epoch();
    let revisions_before: BTreeMap<mush_industry::ClaimId, u64> =
        opened(&mut rig.tw, AHUNA, letter)
            .into_iter()
            .flat_map(|deposit| claims_over(&mut rig.tw, deposit))
            .map(|each| (each.id(), each.revision()))
            .collect();
    assert!(
        !revisions_before.is_empty(),
        "the block holds claims to contest"
    );
    let out = claim_go(&mut rig, b, &format!("+claim/contest {letter} go"));
    assert!(
        out.contains(&format!("You contest block {block} at Ahuna Flank;")),
        "{out}"
    );
    let record = block_record(&mut rig.tw, AHUNA, letter).expect("the record");
    let won = record.claim.expect("the claim");
    assert_eq!(won.holder, mush_org::PrincipalId::Object(b));
    assert_eq!(won.discovered_ic, Some(sounding));
    assert_eq!(won.epoch, epoch_before + 1);
    assert_eq!(
        tenure_of(&mut rig.tw, AHUNA, letter).epoch(),
        epoch_before + 1
    );
    for deposit in opened(&mut rig.tw, AHUNA, letter) {
        for each in claims_over(&mut rig.tw, deposit) {
            assert_eq!(each.holder(), &mush_org::PrincipalId::Object(b));
            assert_eq!(each.discovered_at().map(|at| at.at), Some(sounding));
            // One post-image, one revision (the judge's conformance F7).
            assert_eq!(
                Some(each.revision()),
                revisions_before
                    .get(&each.id())
                    .map(|revision| revision + 1),
                "a contest moves each claim's revision by one"
            );
        }
    }
    let b_fee = won.fee.expect("Bryn's fee");
    let mut heard = String::new();
    for _ in 0..20 {
        let events = fire_landing(&mut rig);
        heard.push_str(&said(&rig.tw, &events, a));
        heard.push_str(&said(&rig.tw, &events, b));
        sweep(&mut rig.tw, 60);
    }
    assert!(
        heard.contains(&format!("passes block {block} at Ahuna Flank to you")),
        "{heard}"
    );
    assert!(
        heard.contains(&format!(
            "Bryn's heard sounding predates your claim on block {block} at Ahuna Flank; the \
             Registry passes the claim to them and returns your last term fee."
        )),
        "{heard}"
    );
    assert!(matches!(
        contract_state(&rig.tw, b_fee),
        ContractState::Settled { .. }
    ));
    assert_eq!(
        available(&rig.tw, a),
        ada_before + 200_000,
        "the term fee came back once"
    );
    let mut rig = reboot(rig, &[a, b]);
    for _ in 0..5 {
        let _ = fire_landing(&mut rig);
    }
    assert_eq!(
        available(&rig.tw, a),
        ada_before + 200_000,
        "and never twice"
    );
    // The clerk is not wedged: a release still lapses and is chronicled.
    let _ = claim_go(&mut rig, b, &format!("+claim/release {letter} go"));
    let _ = settle(&mut rig, &[b]);
    assert_eq!(lapse_lines(&rig.tw, AHUNA, letter, epoch_before + 1), 1);
}

/// **C11b — a re-staked block is still contestable** (reviewer F3, D18): Ada stakes and
/// releases; Bryn sounds loudly after the lapse; Cato reads it later and takes it over; Bryn
/// contests and wins — the window opened at the lapse, not at Cato's take-over.
#[test]
fn sm10_a_re_staked_block_is_still_contestable() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    let (c, _, tc) = on_ahuna(&mut rig, "Cato", "CatoAcct", "Tern");
    for (who, tv) in [(a, ta), (b, tb), (c, tc)] {
        launched(&mut rig, who, tv);
    }
    let letter = ore_block(&mut rig.tw, AHUNA);
    let _ = stake(&mut rig, AHUNA, a, ta, letter);
    let _ = claim_go(&mut rig, a, &format!("+claim/release {letter} go"));
    let _ = settle(&mut rig, &[a]);
    let lapsed = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.lapsed_ic)
        .expect("lapsed");
    let cell = stand_in(&mut rig.tw, AHUNA, letter);
    to_cell_on(&mut rig, AHUNA, tb, cell);
    let _ = sounded(&mut rig, b, tb, false);
    let taken = stake(&mut rig, AHUNA, c, tc, letter);
    assert_eq!(taken.opened_ic, Some(lapsed), "staked against the lapse");
    let out = claim_go(&mut rig, b, &format!("+claim/contest {letter} go"));
    assert!(out.contains("You contest block"), "{out}");
    let won = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .expect("the claim");
    assert_eq!(won.holder, mush_org::PrincipalId::Object(b));
    assert_eq!(won.opened_ic, Some(lapsed), "a contest keeps the opening");
}

/// **C12 — soundings that do not count** (§9.4, D18): a loud sounding from before the block last
/// opened, a quiet one, and a loud one after the holder's stamp each answer
/// `CONTEST_NO_EVIDENCE`; past 30 IC days the contest answers `CONTEST_SHUT`; nothing is
/// escrowed and nothing is submitted.
#[test]
fn sm10_soundings_that_do_not_count() {
    let mut rig = world_on(None);
    let (h, _, th) = on_ahuna(&mut rig, "Hal", "HalAcct", "Salt Moth");
    let (k, _, tk) = on_ahuna(&mut rig, "Kit", "KitAcct", "Tern");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    for (who, tv) in [(h, th), (k, tk), (b, tb)] {
        launched(&mut rig, who, tv);
    }
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let stands = stands_in(&mut rig.tw, AHUNA, letter);
    // A loud sounding before the block last opens.
    to_cell_on(&mut rig, AHUNA, tb, stands[0].0);
    let _ = sounded(&mut rig, b, tb, false);
    let _ = stake(&mut rig, AHUNA, h, th, letter);
    let _ = claim_go(&mut rig, h, &format!("+claim/release {letter} go"));
    let _ = settle(&mut rig, &[h]);
    // A quiet one after it opened.
    to_cell_on(&mut rig, AHUNA, tb, stands[1].0);
    let _ = sounded(&mut rig, b, tb, true);
    let _ = stake(&mut rig, AHUNA, k, tk, letter);
    let none = cl::CONTEST_NO_EVIDENCE.replace("{block}", &block);
    let before = available(&rig.tw, b);
    let (out, queued) = claim(&mut rig, b, &format!("+claim/contest {letter} go"));
    assert_eq!(
        out.trim(),
        none,
        "neither the old loud nor the quiet one counts"
    );
    assert!(queued.is_empty());
    // A loud one after the holder's stamp.
    to_cell_on(&mut rig, AHUNA, tb, stands[2].0);
    let _ = sounded(&mut rig, b, tb, false);
    let (out, queued) = claim(&mut rig, b, &format!("+claim/contest {letter} go"));
    assert_eq!(
        out.trim(),
        none,
        "after the holder's reading counts for nothing"
    );
    assert!(queued.is_empty());
    // Past the window.
    let _ = days_on(&mut rig, 31, &[b]);
    let (out, queued) = claim(&mut rig, b, &format!("+claim/contest {letter} go"));
    assert_eq!(
        out.trim(),
        cl::CONTEST_SHUT
            .replace("{block}", &block)
            .replace("{days}", "30")
    );
    assert!(queued.is_empty());
    assert_eq!(available(&rig.tw, b), before, "nothing escrowed");
}

// ---------------------------------------------------------------------------
// C13, C14 — Occator: the seeds, the paper, the Registrar
// ---------------------------------------------------------------------------

/// `@atlas/pack plan` then `@atlas/pack apply <fingerprint>` on the lane (`commerce.rs`'s).
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

/// The contracts named `name`.
fn contracts_named(tw: &TestWorld, name: &str) -> Vec<mush_economy::EconomicContract> {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
        .contracts()
        .filter(|contract| contract.name.as_str() == name)
        .cloned()
        .collect()
}

/// **C13 — Occator's holdings and the paper are seeded by the clerk** (lead ruling 10, D9): the
/// paper's adjudication contract first (`Offered`, issued by the Syndicate, adjudicated by the
/// Registry, nothing escrowed, a century's deadline) and filed before any seed, then one block a
/// fire, never two — F, G, J, K Occator's `Uncontested` claims under the Registry, the twelve
/// outer blocks the Syndicate's `Contested` paper; `+tv`'s ground line names the holding's org,
/// and the site's tenure reads exactly as authored on every surface; the seeded site's refusals
/// verbatim (`CONTEST_HELD`, `CONTEST_OPEN`, `CLAIM_NOT_YOURS`, `CLAIM_LIST_NONE`,
/// `REGISTRY_UNKNOWN`); a re-apply of the lane, and a reboot (with the claim log deleted, so the
/// contract is re-found), seed nothing twice and file no second contract, and neither the
/// applier nor the boot pass submits anything.
#[test]
fn sm10_occators_holdings_and_the_paper_are_seeded_by_the_clerk() {
    let mut rig = world_on(None);
    let (p, _, tv) = on_pad_at(
        &mut rig,
        ("Pip", "PipAcct", "Salt Moth"),
        ("occator", OCCATOR),
    );
    let clerk = clerk_of(&rig.tw, OCCATOR);
    let name = "Pallas Syndicate paper: Occator Rim";
    let mut actors = Vec::new();
    let mut seeds = 0;
    for _ in 0..40 {
        let (_, queued) = fire(&mut rig);
        let acts = tenure_actors(&queued);
        assert!(acts.len() <= 1, "one block a fire: {acts:?}");
        if !acts.is_empty() {
            assert_eq!(
                contracts_named(&rig.tw, name).len(),
                1,
                "the paper's contract is filed before any block is seeded"
            );
            seeds += 1;
        }
        actors.extend(acts);
        land_all(&mut rig, &queued);
    }
    assert_eq!(seeds, 16, "sixteen seeds, one a fire");
    assert!(
        actors.iter().all(|actor| *actor == clerk),
        "only the clerk seeds"
    );
    let papers = contracts_named(&rig.tw, name);
    assert_eq!(papers.len(), 1, "one paper contract");
    let paper = &papers[0];
    assert_eq!(paper.state, ContractState::Offered, "filed, never accepted");
    assert!(
        paper.acceptances.is_empty() && paper.escrow.is_empty(),
        "nothing escrowed"
    );
    let syndicate = mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("pallas_syndicate")].org,
    ));
    let registry = mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("freeport_registry")].org,
    ));
    assert_eq!(paper.issuer, syndicate);
    assert_eq!(
        paper.consent.adjudicator,
        Some(mush_economy::AttestorSet::Party(registry.clone())),
        "adjudicated by the Registry"
    );
    let deadline = paper.consent.deadline.expect("a deadline");
    assert_eq!(deadline.clock, mush_economy::EconomicClock::WorldIc);
    assert!(
        deadline.at > ic_now(&rig.tw) + 36_000 * DAY,
        "a century out"
    );
    for letter in tacsim::port::def::BLOCK_LETTERS {
        let record = block_record(&mut rig.tw, OCCATOR, letter).expect("seeded");
        let deposits = opened(&mut rig.tw, OCCATOR, letter);
        if ['F', 'G', 'J', 'K'].contains(&letter) {
            let claim = record.claim.expect("Occator's claim");
            assert!(!claim.player_held());
            assert!(record.paper.is_none());
            for deposit in &deposits {
                let claims = claims_over(&mut rig.tw, *deposit);
                assert_eq!(claims.len(), 1);
                assert_eq!(claims[0].issuer(), &registry);
                assert_eq!(
                    claims[0].contest_state(),
                    &mush_industry::ContestState::Uncontested
                );
            }
        } else {
            assert!(record.claim.is_none());
            let paper_row = record.paper.expect("the paper");
            assert_eq!(paper_row.state, PaperState::Contested);
            assert_eq!(paper_row.contract, paper.id.0);
            for deposit in &deposits {
                let claims = claims_over(&mut rig.tw, *deposit);
                assert_eq!(claims.len(), 1);
                assert_eq!(claims[0].issuer(), &syndicate);
                assert_eq!(
                    claims[0].contest_state(),
                    &mush_industry::ContestState::Contested { contract: paper.id }
                );
            }
        }
    }
    // The status line names the holding's org (the record's word and the authored one agree on a
    // seeded holding; the record's own branches are C15's).
    launched(&mut rig, p, tv);
    let cell = stand_in(&mut rig.tw, OCCATOR, 'F');
    to_cell_on(&mut rig, OCCATOR, tv, cell);
    let status = run(&mut rig.tw, p, "+tv");
    assert!(
        status.contains("Block F (held by Occator Water & Propellant)"),
        "{status}"
    );
    // The seeded holdings read exactly as authored on every surface (lead note 0a).
    let walker = pilot_on(&mut rig, "Wren", "WrenAcct");
    assert_eq!(
        tenure_read(&mut rig, walker, p, OCCATOR, "Occator Rim"),
        "12 open; 4 held by Occator Water & Propellant"
    );
    // The refusals a seeded site answers, verbatim.
    for (line, want) in [
        (
            "+claim/contest F",
            cl::CONTEST_HELD
                .replace("{block}", "F")
                .replace("{org}", "Occator Water & Propellant"),
        ),
        ("+claim/contest A", cl::CONTEST_OPEN.replace("{block}", "A")),
        (
            "+claim/renew C",
            cl::CLAIM_NOT_YOURS
                .replace("{block}", "C")
                .replace("{site}", "Occator Rim"),
        ),
        ("+claim/list", cl::CLAIM_LIST_NONE.to_owned()),
        (
            "+registry nowhere",
            cl::REGISTRY_UNKNOWN.replace("{arg}", "nowhere"),
        ),
    ] {
        let (out, queued) = claim(&mut rig, p, line);
        assert_eq!(out.trim(), want, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    // A re-apply of the lane seeds nothing and files nothing (the applier submits nothing).
    let _ = plan_and_apply(&mut rig.tw);
    let queued = drain(&mut rig.tw);
    assert!(
        tenure_actors(&queued).is_empty(),
        "the applier submits nothing"
    );
    for _ in 0..10 {
        let (_, queued) = fire(&mut rig);
        assert!(tenure_actors(&queued).is_empty(), "nothing seeded twice");
        land_all(&mut rig, &queued);
    }
    assert_eq!(contracts_named(&rig.tw, name).len(), 1, "no second paper");
    // A reboot with the claim log gone seeds nothing twice and files no second paper.
    let mut rig = reboot_without(rig, &format!("tacsim:claimlog:{OCCATOR}"), &[p]);
    let queued = drain(&mut rig.tw);
    assert!(
        tenure_actors(&queued).is_empty(),
        "the boot pass submits nothing"
    );
    let mut actors = Vec::new();
    for _ in 0..10 {
        let (_, queued) = fire(&mut rig);
        actors.extend(tenure_actors(&queued));
        land_all(&mut rig, &queued);
    }
    assert!(actors.is_empty(), "nothing is seeded twice");
    assert_eq!(
        contracts_named(&rig.tw, name).len(),
        1,
        "the contract is re-found"
    );
}

/// C14's world: Pip on Occator Rim's pad (every family rich), the seeds settled, Pip's rig on a
/// papered outer block B, staked.
fn papered_world() -> (Rig, ObjId, ObjId) {
    let mut rig = world_on(Some(ALL_RICH));
    let (p, _, tv) = on_pad_at(
        &mut rig,
        ("Pip", "PipAcct", "Salt Moth"),
        ("occator", OCCATOR),
    );
    let _ = settle(&mut rig, &[p]);
    launched(&mut rig, p, tv);
    let _ = stake(&mut rig, OCCATOR, p, tv, 'B');
    (rig, p, tv)
}

/// **`+registry occator rim` with Pip's claim on B, verbatim** (lead ruling 1; the brief's
/// 3.6 `registry_cmd`): the head; A–P in order — the twelve outer blocks' paper rows, B's claim
/// row (checked by its fixed parts; its stamps move) then B's paper row in `paper` state, the
/// four holdings' `REGISTRY_HELD` rows; the tail.
fn assert_occator_registry(text: &str, paper: &str) {
    let lines: Vec<&str> = text.lines().collect();
    let mut want: Vec<String> = vec![
        cl::REGISTRY_HEAD
            .replace("{issuer}", REGISTRY)
            .replace("{site}", "Occator Rim"),
    ];
    let row = |letter: char, state: &str| {
        cl::REGISTRY_PAPER
            .replace("{block}", &letter.to_string())
            .replace("{paper}", "Pallas Syndicate")
            .replace("{state}", state)
    };
    for letter in tacsim::port::def::BLOCK_LETTERS {
        match letter {
            'F' | 'G' | 'J' | 'K' => want.push(
                cl::REGISTRY_HELD
                    .replace("{block}", &letter.to_string())
                    .replace("{org}", "Occator Water & Propellant"),
            ),
            'B' => {
                want.push("<claim>".to_owned());
                want.push(row('B', paper));
            }
            _ => want.push(row(letter, "contested")),
        }
    }
    want.push(
        cl::REGISTRY_TAIL
            .replace("{open}", "11")
            .replace("{commons}", "0"),
    );
    assert_eq!(lines.len(), want.len(), "{text}");
    for (got, want) in lines.iter().zip(&want) {
        if want == "<claim>" {
            assert!(
                got.starts_with("  Block B: Pip; term to ")
                    && got.contains("; epoch 0; no family declared; first reading "),
                "{got}"
            );
        } else {
            assert_eq!(got, want, "{text}");
        }
    }
}

/// **C14a — the Registrar suspends the paper at 100 t** (§9.6, lead ruling 11): Pip works 100 t
/// of papered block B by real cycles and renews; the paper's claims read `Suspended { by:
/// Registry }`, the block record's paper `Suspended`, the claim renewed, `RENEW_RECORDED` with
/// `REGISTRAR_TOLD`; every tenure act that landed is Pip's or the clerk's; `+registry` lists the
/// site verbatim before (B's paper `contested`) and after (`suspended by the Freeport Registry`),
/// and Pip's second stake of B answers `STAKE_YOURS`.
#[test]
fn sm10_the_registrar_suspends_the_paper_at_100_t() {
    let (mut rig, p, tv) = papered_world();
    let clerk = clerk_of(&rig.tw, OCCATOR);
    let worked = work(&mut rig, OCCATOR, p, tv, 'B', 100);
    let before = block_record(&mut rig.tw, OCCATOR, 'B').expect("B");
    let held = before.claim.clone().expect("Pip's claim");
    assert_occator_registry(&run(&mut rig.tw, p, "+registry occator rim"), "contested");
    // The holder's own block refuses a second stake (`STAKE_YOURS`).
    let (out, queued) = claim(&mut rig, p, "+claim/stake go");
    assert_eq!(out.trim(), cl::STAKE_YOURS.replace("{block}", "B"));
    assert!(queued.is_empty());
    let (preview, queued) = claim(&mut rig, p, "+claim/renew B");
    assert!(queued.is_empty());
    assert!(
        preview.contains(&format!("worked {worked} of 100 t this term"))
            && preview.contains(
                " The Registrar suspends the Pallas Syndicate's paper with this renewal."
            ),
        "{preview}"
    );
    let (out, queued) = command(&mut rig, p, "+claim/renew B go");
    assert!(out.contains("You renew block B at Occator Rim;"), "{out}");
    assert_eq!(tenure_actors(&queued), vec![p], "the pilot's own act");
    land_all(&mut rig, &queued);
    let mut actors = Vec::new();
    let mut heard = String::new();
    for _ in 0..6 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, p));
        actors.extend(tenure_actors(&queued));
        land_all(&mut rig, &queued);
    }
    assert!(actors.iter().all(|actor| *actor == clerk || *actor == p));
    assert_occator_registry(
        &run(&mut rig.tw, p, "+registry occator rim"),
        &format!("suspended by the {REGISTRY}"),
    );
    let after = block_record(&mut rig.tw, OCCATOR, 'B').expect("B");
    assert_eq!(
        after.paper.as_ref().map(|paper| paper.state),
        Some(PaperState::Suspended)
    );
    let renewed = after.claim.expect("the claim");
    assert_eq!(
        renewed.term_start_ic, held.term_end_ic,
        "renewed from its end"
    );
    let registry = mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("freeport_registry")].org,
    ));
    let syndicate = mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("pallas_syndicate")].org,
    ));
    for deposit in opened(&mut rig.tw, OCCATOR, 'B') {
        for each in claims_over(&mut rig.tw, deposit) {
            if each.issuer() == &syndicate {
                assert_eq!(
                    each.contest_state(),
                    &mush_industry::ContestState::Suspended {
                        by: registry.clone()
                    }
                );
            } else {
                assert_eq!(each.term().at, renewed.term_end_ic);
            }
        }
    }
    assert!(
        heard.contains("The Freeport Registry renews your claim on block B at Occator Rim until ")
            && heard
                .contains(" It suspends the Pallas Syndicate's paper there: the ground is worked."),
        "{heard}"
    );
}

/// **C14b — under 100 t the paper stands** (§9.6): Pip works one hopper of papered block B and
/// asks to renew: `RENEW_PAPER_SHORT`, no submission, the paper `Contested`.
#[test]
fn sm10_under_100_t_the_paper_stands() {
    let (mut rig, p, tv) = papered_world();
    let worked = work(&mut rig, OCCATOR, p, tv, 'B', 10);
    let held = block_record(&mut rig.tw, OCCATOR, 'B')
        .and_then(|record| record.claim)
        .expect("the claim");
    let (out, queued) = claim(&mut rig, p, "+claim/renew B go");
    assert_eq!(
        out.trim(),
        cl::RENEW_PAPER_SHORT
            .replace("{block}", "B")
            .replace("{site}", "Occator Rim")
            .replace("{worked}", &worked.to_string())
            .replace("{floor}", "100")
            .replace("{paper}", "Pallas Syndicate")
            .replace(
                "{until}",
                &tacsim::port::licence::ic_stamp(held.term_end_ic)
            )
    );
    assert!(queued.is_empty());
    assert_eq!(
        block_record(&mut rig.tw, OCCATOR, 'B').and_then(|record| record.paper.map(|p| p.state)),
        Some(PaperState::Contested)
    );
}

// ---------------------------------------------------------------------------
// C15, C16 — the map and the books
// ---------------------------------------------------------------------------

/// **C15 — claims on the site map, and the block's tenure on every surface** (lead ruling 12,
/// D13; §3.8 item 20; lead note 0a): no claim, no `site.claims` key, no `#`, `+registry` reads
/// `REGISTRY_NONE` and Ahuna's tenure reads "16 open" on `+chart`, the `sky.chart` feed, the
/// `body.ground` frame and `+chart/ground`; once Ada holds block C, `#` stands on C's edge
/// (never on its letter's cell) the same for Ada and Bryn, `site.claims` names it with `yours`
/// true only for Ada, the rig's ground line reads `TENURE_YOURS` for Ada and `TENURE_CLAIMED`
/// for Bryn standing in the same block, and the site's tenure reads "15 open; 1 claimed"
/// everywhere.
#[test]
fn sm10_claims_on_the_site_map() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    let walker = pilot_on(&mut rig, "Wren", "WrenAcct");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let before = map_frame(&mut rig, a, json!({ "view": "site" }));
    assert!(
        before["site"].get("claims").is_none(),
        "omitted with no claim"
    );
    assert!(claim_cells(&before).is_empty());
    assert_eq!(
        run(&mut rig.tw, b, "+registry ahuna flank"),
        format!(
            "{}\n{}",
            cl::REGISTRY_HEAD
                .replace("{issuer}", REGISTRY)
                .replace("{site}", "Ahuna Flank"),
            cl::REGISTRY_NONE
        )
    );
    assert_eq!(
        tenure_read(&mut rig, walker, a, AHUNA, "Ahuna Flank"),
        "16 open"
    );
    let _ = stake(&mut rig, AHUNA, a, ta, 'C');
    // The rig's ground line reads the record: Ada's own, Bryn's view of Ada's.
    let status = run(&mut rig.tw, a, "+tv");
    assert!(
        status.contains(&format!("Block C ({})", cl::TENURE_YOURS)),
        "{status}"
    );
    let cell = stand_in(&mut rig.tw, AHUNA, 'C');
    to_cell_on(&mut rig, AHUNA, tb, cell);
    let status = run(&mut rig.tw, b, "+tv");
    assert!(
        status.contains(&format!(
            "Block C ({})",
            cl::TENURE_CLAIMED.replace("{holder}", "Ada")
        )),
        "{status}"
    );
    assert_eq!(
        tenure_read(&mut rig, walker, a, AHUNA, "Ahuna Flank"),
        "15 open; 1 claimed"
    );
    let mine = map_frame(&mut rig, a, json!({ "view": "site" }));
    let theirs = map_frame(&mut rig, b, json!({ "view": "site" }));
    let cells = claim_cells(&mine);
    assert!(!cells.is_empty());
    assert_eq!(
        cells,
        claim_cells(&theirs),
        "the same grid for every viewer"
    );
    // C is the third block of the northern row: frame cells x 16..24, y 24..32 in the site view;
    // its letter stands at its north-west cell (16, 31).
    assert!(
        cells
            .iter()
            .all(|(x, y)| (16..24).contains(x) && (24..32).contains(y)),
        "{cells:?}"
    );
    assert!(!cells.contains(&(16, 31)), "never over the letter");
    let rows = frame_rows(&mine);
    assert_eq!(rows[31].chars().nth(16), Some('C'));
    assert_eq!(
        mine["site"]["claims"],
        json!([{ "letter": "C", "state": "claimed", "holder": "Ada", "yours": true }])
    );
    assert_eq!(
        theirs["site"]["claims"],
        json!([{ "letter": "C", "state": "claimed", "holder": "Ada", "yours": false }])
    );
}

/// The registry and the list for Ada's stake of C in one world, aligned.
fn c16_world(table: &'static str) -> (String, String) {
    let mut rig = world_with_table(table);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    // Doc 20 cut SM.11: aligned before the launches, so the jump to the aligned instant never
    // leaves a rig a day idle on the ground (the idle return would bring it home).
    align(&mut rig);
    launched(&mut rig, a, ta);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let _ = claim_go(&mut rig, a, "+claim/stake metals go");
    let _ = settle(&mut rig, &[a]);
    (
        run(&mut rig.tw, a, "+registry ahuna flank"),
        run(&mut rig.tw, a, "+claim/list"),
    )
}

/// **C16 — the registry and the list print public facts** (lead ruling 1): the rows verbatim;
/// "worked 32 of 50 t" after 32 t; the same `+registry` and `+claim/list` for the same stake in
/// the three deposit-count worlds.
#[test]
fn sm10_the_registry_and_the_list_print_public_facts() {
    let rich = c16_world(ALL_RICH);
    assert_eq!(
        rich,
        c16_world(METALS_ONLY),
        "four deposits and one read alike"
    );
    assert_eq!(rich, c16_world(ALL_BARREN), "and none");
    let (registry, list) = rich;
    let lines: Vec<&str> = registry.lines().collect();
    assert_eq!(lines[0], "The Freeport Registry's claims at Ahuna Flank:");
    assert!(
        lines[1].starts_with("  Block C: Ada; term to ")
            && lines[1].contains("; epoch 0; declared metals; first reading ")
            && lines[1].contains("; worked 0 of 50 t; contestable to "),
        "{registry}"
    );
    assert_eq!(lines[2], "  15 open, 0 commons, unclaimed.");
    assert!(
        list.starts_with("Your claims (1 of 2):\n  Ahuna Flank block C: term to "),
        "{list}"
    );
    // 32 t worked.
    let mut rig = world_on(Some(ALL_RICH));
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let _ = stake(&mut rig, AHUNA, a, ta, 'C');
    let worked = work(&mut rig, AHUNA, a, ta, 'C', 32);
    let registry = run(&mut rig.tw, a, "+registry ahuna flank");
    assert!(
        registry.contains(&format!("worked {worked} of 50 t")),
        "{registry}"
    );
    let list = run(&mut rig.tw, a, "+claim/list");
    assert!(
        list.contains(&format!("worked {worked} of 50 t.")),
        "{list}"
    );
}

// ---------------------------------------------------------------------------
// C17–C22 — money and drift
// ---------------------------------------------------------------------------

/// **C17 — a refused stake refunds its escrow** (OC14): Ada and Bryn `go` on one block in one
/// step; Ada's lands; Bryn's offer re-plans to "under claim" and offers nothing, so nothing
/// lands for him; his `Offered` fee refunds at its deadline, and nothing is paid twice.
#[test]
fn sm10_a_refused_stake_refunds_its_escrow() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let _ = scanned_in(&mut rig, AHUNA, b, tb, letter);
    let (a_before, b_before) = (available(&rig.tw, a), available(&rig.tw, b));
    let (_, a_queued) = claim(&mut rig, a, "+claim/stake go");
    let (out, b_queued) = claim(&mut rig, b, "+claim/stake go");
    assert!(out.contains("You stake block"), "{out}");
    let b_fee = cl::claim_order_for_tests(rig.tw.world(), b).expect("Bryn's order");
    land_all(&mut rig, &a_queued);
    let refused = land(&mut rig, &b_queued[0]).expect_err("Bryn's offer is gone");
    assert!(refused.contains("not currently offered"), "{refused}");
    let _ = settle(&mut rig, &[a, b]);
    for _ in 0..70 {
        sweep(&mut rig.tw, 60);
    }
    assert!(matches!(
        contract_state(&rig.tw, b_fee),
        ContractState::Expired | ContractState::Cancelled
    ));
    assert_eq!(available(&rig.tw, b), b_before, "Bryn's escrow came back");
    assert_eq!(available(&rig.tw, a), a_before - 400_000, "Ada paid once");
}

/// **A lost race never locks the account out** (the fixer's round, correctness F6): Bryn holds
/// one claim; he and Ada `go` on one block in one step and Ada's lands, so Bryn's offer is gone
/// and his fee waits `Offered` for its deadline. While his order is young it still counts
/// (`CLAIM_CAP` on another block); once it is older than `CLAIM_ORDER_TTL_IC_SECS` — never
/// offered again, its fee merely refunding — it no longer does, and his stake of the other
/// block files.
#[test]
fn sm10_a_lost_race_never_locks_the_account_out() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let _ = stake(&mut rig, AHUNA, b, tb, 'A');
    let e_cell = scanned_in(&mut rig, AHUNA, b, tb, 'E');
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let _ = scanned_in(&mut rig, AHUNA, b, tb, 'C');
    let (_, a_queued) = claim(&mut rig, a, "+claim/stake go");
    let (_, b_queued) = claim(&mut rig, b, "+claim/stake go");
    let lost = cl::claim_order_for_tests(rig.tw.world(), b).expect("Bryn's order");
    land_all(&mut rig, &a_queued);
    assert!(
        land(&mut rig, &b_queued[0]).is_err(),
        "Bryn's offer is gone"
    );
    to_cell_on(&mut rig, AHUNA, tb, e_cell);
    let (out, queued) = claim(&mut rig, b, "+claim/stake go");
    assert_eq!(
        out.trim(),
        cl::CLAIM_CAP.replace("{max}", "2"),
        "a young order counts"
    );
    assert!(queued.is_empty());
    let past_ttl = ic_now(&rig.tw) + tacsim::surface::tenure::CLAIM_ORDER_TTL_IC_SECS + 60;
    let _ = to_ic(&mut rig, past_ttl);
    assert_eq!(
        contract_state(&rig.tw, lost),
        ContractState::Offered,
        "the lost fee still waits for its deadline"
    );
    let out = claim_go(&mut rig, b, "+claim/stake go");
    assert!(out.contains("You stake block E"), "{out}");
}

/// **C18 — a restart between landing and acceptance still takes the fee**: the stake lands,
/// the world restarts before any fire, and the clerk then accepts the fee and the economy settles
/// it.
#[test]
fn sm10_a_restart_between_landing_and_acceptance_still_takes_the_fee() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    assert_eq!(contract_state(&rig.tw, fee), ContractState::Offered);
    let mut rig = reboot(rig, &[a]);
    let heard = settle(&mut rig, &[a]);
    assert!(heard.contains("records your claim on block"), "{heard}");
    for _ in 0..30 {
        if matches!(contract_state(&rig.tw, fee), ContractState::Settled { .. }) {
            break;
        }
        sweep(&mut rig.tw, 60);
    }
    assert!(matches!(
        contract_state(&rig.tw, fee),
        ContractState::Settled { .. }
    ));
}

/// **C19 — a fee cancelled after landing lapses the claim** (reviewer F2a, D19, OC16): Ada's
/// stake lands and she cancels its fee contract before the clerk fires; the clerk lapses the
/// claim unpaid (epoch + 1, the cooling row, `UNPAID_LAPSED`) and no money moved.
#[test]
fn sm10_a_fee_cancelled_after_landing_lapses_the_claim() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let (before, registry) = (available(&rig.tw, a), registry_balance(&rig.tw));
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    let out = run(&mut rig.tw, a, &format!("@contract/cancel #{fee}"));
    assert_eq!(
        contract_state(&rig.tw, fee),
        ContractState::Cancelled,
        "{out}"
    );
    let heard = settle(&mut rig, &[a]);
    assert!(
        heard.contains(
            &cl::UNPAID_LAPSED
                .replace("{block}", &block)
                .replace("{site}", "Ahuna Flank")
        ),
        "{heard}"
    );
    let record = block_record(&mut rig.tw, AHUNA, letter).expect("the record");
    assert!(record.claim.and_then(|claim| claim.lapsed_ic).is_some());
    let tenure = tenure_of(&mut rig.tw, AHUNA, letter);
    assert_eq!(tenure.epoch(), 1);
    assert_eq!(tenure.cooling().len(), 1);
    assert_eq!(available(&rig.tw, a), before, "no money moved");
    assert_eq!(registry_balance(&rig.tw), registry);
}

/// **C20 — nothing follows a pending fee** (reviewer F2b): while Ada's fee is still `Offered`,
/// her renewal and her release and Bryn's contest each answer `CLAIM_PENDING` and escrow
/// nothing; once the clerk records it, the release is accepted.
#[test]
fn sm10_nothing_follows_a_pending_fee() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let cell = stand_in(&mut rig.tw, AHUNA, letter);
    to_cell_on(&mut rig, AHUNA, tb, cell);
    let _ = sounded(&mut rig, b, tb, false);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let pending = cl::CLAIM_PENDING
        .replace("{block}", &letter.to_string())
        .replace("{site}", "Ahuna Flank");
    let (a_before, b_before) = (available(&rig.tw, a), available(&rig.tw, b));
    for (who, line) in [
        (a, format!("+claim/renew {letter} go")),
        (a, format!("+claim/release {letter} go")),
        (b, format!("+claim/contest {letter} go")),
    ] {
        let (out, queued) = claim(&mut rig, who, &line);
        assert_eq!(out.trim(), pending, "{line}");
        assert!(queued.is_empty());
    }
    assert_eq!(
        (available(&rig.tw, a), available(&rig.tw, b)),
        (a_before, b_before)
    );
    let _ = settle(&mut rig, &[a]);
    let out = claim_go(&mut rig, a, &format!("+claim/release {letter} go"));
    assert!(out.contains("You release block"), "{out}");
}

/// **An outage past a fee's real hour**: one leap of the clock at world rate 1 (so the IC day
/// barely moves), everything due fired once, then whatever the leap queued landed.
fn outage(rig: &mut Rig) {
    set_rate(&mut rig.tw, 1);
    sweep(
        &mut rig.tw,
        tacsim::surface::tenure::CLAIM_FEE_WINDOW_SECS + 60,
    );
    set_rate(&mut rig.tw, RATE);
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
}

/// **C20b — a fee expired unaccepted after landing lapses the claim and cools nobody** (the
/// lead's ruling on R1, OC16; the judge's conformance F5). Reached through the never-paid
/// arm's seam (a staff-hand record edit, as `sm10_a_refund_waits_for_the_losers_fee_and_never_mints`
/// does): Ada files a stake that never lands and the world runs past its fee's real hour, so
/// that fee expires unaccepted and its escrow comes back (C17's path); her next stake lands and,
/// before any fire of the clerk's, staff point the landed claim's fee at the expired contract —
/// the state an outage leaves when the economy's expiry reaches a landed claim's fee before the
/// clerk's next visit. Until the clerk's lapse lands her renewal and her release answer
/// `CLAIM_PENDING` (C20's rule); then the clerk lapses it unpaid (the unpaid lapse's receipt,
/// the clerk's act): `lapsed_ic`, epoch + 1, one canon line, `EXPIRED_LAPSED` told once — and
/// **no** cooling term, unlike the withdrawn fee (C19): no cooling row at all, so a later clock
/// generation (the second outage below changes the world rate) reads nothing as live. No money
/// moved, and after a fresh reading her own stake of the block files again at once.
#[test]
#[allow(clippy::too_many_lines)]
fn sm10_a_fee_expired_unaccepted_after_landing_lapses_the_claim_and_cools_nobody() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let other = if letter == 'M' { 'P' } else { 'M' };
    let (before, registry) = (available(&rig.tw, a), registry_balance(&rig.tw));
    // A stake that never lands: its fee waits Offered, unreferenced, and expires unaccepted.
    let _ = scanned_in(&mut rig, AHUNA, a, ta, other);
    let (_, dropped) = claim(&mut rig, a, "+claim/stake go");
    assert_eq!(dropped.len(), 1, "one order queued, never landed");
    let expired = cl::claim_order_for_tests(rig.tw.world(), a).expect("the order");
    outage(&mut rig);
    assert_eq!(contract_state(&rig.tw, expired), ContractState::Expired);
    assert_eq!(available(&rig.tw, a), before, "its escrow came back");
    // Her stake that lands, and the staff hand: the landed claim's fee is the expired one.
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let mut record = block_record(&mut rig.tw, AHUNA, letter).expect("the record");
    let filed = record.claim.as_mut().expect("Ada's claim");
    let unused = filed.fee.replace(expired).expect("the landed fee");
    put_block(&mut rig, record);
    let on_block = format!("/{AHUNA}/{}/", letter.to_ascii_lowercase());
    let tenure_on_block = |queued: &[AuthoritativeSubmit]| -> Vec<(ObjId, String)> {
        queued
            .iter()
            .filter(|submit| {
                submit.action.key().id.as_str() == mush_industry::ACTION_CLAIM_TENURE
                    && submit.key.as_str().contains(&on_block)
            })
            .map(|submit| (submit.actor, submit.key.as_str().to_owned()))
            .collect()
    };
    // Before the clerk's visit, C20's rule stands.
    let pending = cl::CLAIM_PENDING
        .replace("{block}", &block)
        .replace("{site}", "Ahuna Flank");
    for line in [
        format!("+claim/renew {letter} go"),
        format!("+claim/release {letter} go"),
    ] {
        let (out, queued) = claim(&mut rig, a, &line);
        assert_eq!(out.trim(), pending, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    // The clerk's lapse: the unpaid lapse's receipt, the clerk's act, and nothing else.
    let mut heard = String::new();
    let mut lapses = Vec::new();
    for _ in 0..40 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, a));
        heard.push('\n');
        lapses.extend(tenure_on_block(&queued));
        land_all(&mut rig, &queued);
        if block_record(&mut rig.tw, AHUNA, letter)
            .and_then(|record| record.claim)
            .is_some_and(|claim| claim.lapsed_ic.is_some())
            && heard.contains("lapsed unpaid")
        {
            break;
        }
    }
    heard.push_str(&settle(&mut rig, &[a]));
    let unpaid_key = format!(
        "tacsim.claim.unpaid/{AHUNA}/{}/0",
        letter.to_ascii_lowercase()
    );
    let clerk = clerk_of(&rig.tw, AHUNA);
    assert!(
        !lapses.is_empty()
            && lapses
                .iter()
                .all(|(actor, key)| *actor == clerk && *key == unpaid_key),
        "the clerk's unpaid lapse, and nothing else, acts on the block: {lapses:?}"
    );
    let told = cl::EXPIRED_LAPSED
        .replace("{block}", &block)
        .replace("{site}", "Ahuna Flank");
    assert_eq!(heard.matches(&told).count(), 1, "told once: {heard}");
    assert!(
        !heard.contains(
            &cl::UNPAID_LAPSED
                .replace("{block}", &block)
                .replace("{site}", "Ahuna Flank")
        ),
        "{heard}"
    );
    let record = block_record(&mut rig.tw, AHUNA, letter).expect("the record");
    assert!(record.claim.and_then(|claim| claim.lapsed_ic).is_some());
    let tenure = tenure_of(&mut rig.tw, AHUNA, letter);
    assert_eq!(tenure.epoch(), 1, "the lapse opened an epoch");
    assert!(
        tenure.cooling().is_empty(),
        "no cooling term, not even a row that has run out: {tenure:?}"
    );
    assert_eq!(lapse_lines(&rig.tw, AHUNA, letter, 0), 1);
    // The landed stake's own fee, never referenced again, refunds at its deadline.
    outage(&mut rig);
    assert_eq!(contract_state(&rig.tw, unused), ContractState::Expired);
    assert_eq!(available(&rig.tw, a), before, "no money moved");
    assert_eq!(
        registry_balance(&rig.tw),
        registry,
        "the Registry took nothing"
    );
    // The block reopened at once: a fresh reading, a preview with no cooling term, a stake.
    recharge(&mut rig, ta);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let (preview, queued) = claim(&mut rig, a, "+claim/stake");
    assert!(queued.is_empty());
    assert!(
        preview.starts_with(&format!("Block {block}, your readings: ")),
        "{preview}"
    );
    let out = claim_go(&mut rig, a, "+claim/stake go");
    assert!(
        out.contains(&format!("You stake block {block} at Ahuna Flank;")),
        "{out}"
    );
    let _ = settle(&mut rig, &[a]);
    let again = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .expect("Ada's claim again");
    assert_eq!(again.holder, mush_org::PrincipalId::Object(a));
    assert_eq!(again.lapsed_ic, None);
    assert_eq!(again.epoch, 2, "the take-over opened the next epoch");
}

/// **C20c — a fee the Registry accepted and an outage then expired is forfeit to it: the claim
/// stands paid** (the lead's ruling on R1, OC16 lapses only the fee that expired
/// *unaccepted*): Ada's stake lands and the world goes dark past the fee's real hour — one leap
/// of the clock at world rate 1 — in which the clerk's fire accepts the fee before the economy's
/// expiry reaches it, so the bound contract is forfeit to the Registry's account
/// (`Expiry::Forfeit`). The Registry holds the fee: nothing lapses the claim, no lapse is told,
/// and her acts no longer wait on it — her release, never `CLAIM_PENDING`, lands.
#[test]
fn sm10_a_fee_forfeit_to_the_registry_after_an_outage_is_paid() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let (before, registry) = (available(&rig.tw, a), registry_balance(&rig.tw));
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    assert_eq!(contract_state(&rig.tw, fee), ContractState::Offered);
    outage(&mut rig);
    assert_eq!(contract_state(&rig.tw, fee), ContractState::Expired);
    assert_eq!(
        registry_balance(&rig.tw),
        registry + 400_000,
        "accepted, then forfeit to the Registry: the fee is paid"
    );
    assert_eq!(available(&rig.tw, a), before - 400_000);
    let heard = settle(&mut rig, &[a]);
    assert!(
        !heard.contains("lapsed unpaid") && !heard.contains("was withdrawn"),
        "{heard}"
    );
    let standing = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .expect("Ada's claim");
    assert_eq!(standing.lapsed_ic, None, "the claim stands");
    assert_eq!(tenure_of(&mut rig.tw, AHUNA, letter).epoch(), 0);
    let out = claim_go(&mut rig, a, &format!("+claim/release {letter} go"));
    assert!(
        out.contains(&format!("You release block {block} at Ahuna Flank.")),
        "{out}"
    );
}

/// The economy's transactions the site clerk posted as contest refunds (`CLAIM_REFUND_KEY`).
fn refund_posts(tw: &TestWorld) -> Vec<mush_economy::EconomicTransaction> {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .transactions()
        .filter(|tx| {
            matches!(
                &tx.cause,
                mush_economy::EconomicCause::Internal { kind, .. }
                    if kind.as_str() == tacsim::port::site::CLAIM_REFUND_KEY
            )
        })
        .cloned()
        .collect()
}

/// **C20d — a fee forfeit to the Registry is a fee the loser paid** (the reviewer's F1 on the
/// lead's ruling on R1, OC16: every place that asks whether a fee was paid agrees). Bryn sounds
/// the block loudly; Ada reads it and stakes it, and an outage forfeits her accepted fee to the
/// Registry (C20c). Her claim stands paid, so Bryn's contest is not held `CLAIM_PENDING`: it
/// lands, and the clerk tells Ada `CONTEST_LOST` (never `CONTEST_LOST_UNPAID`) and returns her
/// term fee once under `CLAIM_REFUND_KEY` — the Registry's account debited by exactly the term
/// fee, Ada's credited by it — and a reboot posts nothing twice.
#[test]
#[allow(clippy::too_many_lines)]
fn sm10_a_fee_forfeit_to_the_registry_is_refunded_when_contested() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let cell = stand_in(&mut rig.tw, AHUNA, letter);
    to_cell_on(&mut rig, AHUNA, tb, cell);
    let _ = sounded(&mut rig, b, tb, false);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let a_fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("Ada's fee");
    assert_eq!(contract_state(&rig.tw, a_fee), ContractState::Offered);
    outage(&mut rig);
    assert_eq!(contract_state(&rig.tw, a_fee), ContractState::Expired);
    let _ = settle(&mut rig, &[a]);
    let standing = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .expect("Ada's claim");
    assert_eq!(standing.lapsed_ic, None, "the forfeit claim stands paid");
    let term_minor = 200_000;
    let (ada_before, bryn_before, registry_before) = (
        available(&rig.tw, a),
        available(&rig.tw, b),
        registry_balance(&rig.tw),
    );
    let out = claim_go(&mut rig, b, &format!("+claim/contest {letter} go"));
    assert!(
        out.contains(&format!("You contest block {block} at Ahuna Flank;")),
        "the forfeit fee is paid, so the claim is contestable: {out}"
    );
    let won = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .expect("Bryn's claim");
    assert_eq!(won.holder, mush_org::PrincipalId::Object(b));
    assert_eq!(
        won.refund
            .as_ref()
            .map(|refund| (refund.to, refund.paid_by)),
        Some((a, a_fee)),
        "the refund owed is Ada's forfeit fee"
    );
    let b_fee = won.fee.expect("Bryn's fee");
    let bryn_fee = bryn_before - available(&rig.tw, b);
    assert!(bryn_fee > 0, "Bryn's contest fee escrowed");
    let mut heard = String::new();
    for _ in 0..30 {
        let events = fire_landing(&mut rig);
        heard.push_str(&said(&rig.tw, &events, a));
        sweep(&mut rig.tw, 60);
        if matches!(
            contract_state(&rig.tw, b_fee),
            ContractState::Settled { .. }
        ) && refunded_epoch(&rig.tw, AHUNA, letter) == Some(won.epoch)
        {
            break;
        }
    }
    let _ = settle(&mut rig, &[a]);
    assert!(
        heard.contains(&format!(
            "Bryn's heard sounding predates your claim on block {block} at Ahuna Flank; the \
             Registry passes the claim to them and returns your last term fee."
        )),
        "told CONTEST_LOST: {heard}"
    );
    assert!(
        !heard.contains(&format!(
            "Bryn's heard sounding predates your claim on block {block} at Ahuna Flank; the \
             Registry passes the claim to them."
        )),
        "never CONTEST_LOST_UNPAID: {heard}"
    );
    assert!(matches!(
        contract_state(&rig.tw, b_fee),
        ContractState::Settled { .. }
    ));
    assert_eq!(refunded_epoch(&rig.tw, AHUNA, letter), Some(won.epoch));
    let refunds = refund_posts(&rig.tw);
    assert_eq!(refunds.len(), 1, "one refund: {refunds:?}");
    let registry_account = port_map(&rig.tw, "ceres_port").orgs[&cid("freeport_registry")]
        .account
        .expect("the Registry's account");
    let ada_account = credit_of(&rig.tw, a);
    let moved: BTreeMap<u64, i64> = refunds[0]
        .lines
        .iter()
        .map(|line| (line.account.0, line.delta.minor()))
        .collect();
    assert_eq!(
        moved,
        BTreeMap::from([(registry_account, -term_minor), (ada_account, term_minor)]),
        "the Registry pays Ada exactly the term fee"
    );
    assert_eq!(
        available(&rig.tw, a),
        ada_before + term_minor,
        "the forfeit term fee came back once"
    );
    assert_eq!(
        registry_balance(&rig.tw),
        registry_before + bryn_fee - term_minor,
        "the Registry gained Bryn's fee and fell by exactly the term fee"
    );
    let mut rig = reboot(rig, &[a, b]);
    for _ in 0..5 {
        let _ = fire_landing(&mut rig);
    }
    assert_eq!(
        available(&rig.tw, a),
        ada_before + term_minor,
        "never twice"
    );
    assert_eq!(refund_posts(&rig.tw).len(), 1);
}

/// **A lost claim log tells no lapse twice** (the judge's conformance F9): Ada releases a
/// claim; the clerk chronicles the lapse and tells her `LAPSED_TOLD` once. The world reboots
/// with the site's claim log corrupted, so it loads `Unreadable` (dropped loudly; it bars
/// nothing) and the clerk's chronicle step finds the lapse with no row of its own: the canon
/// record guards the line **and** the tell — across every later fire and a second reboot she
/// is told nothing more, the chronicle holds one line, and the clerk's next write replaces the
/// log with the row.
#[test]
fn sm10_a_lost_claim_log_tells_no_lapse_twice() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let held = stake(&mut rig, AHUNA, a, ta, letter);
    let out = claim_go(&mut rig, a, &format!("+claim/release {letter} go"));
    assert!(out.contains("You release block"), "{out}");
    let told = format!("Your claim on block {block} at Ahuna Flank has lapsed;");
    let mut heard = settle(&mut rig, &[a]);
    assert_eq!(heard.matches(&told).count(), 1, "told once: {heard}");
    assert_eq!(lapse_lines(&rig.tw, AHUNA, letter, held.epoch), 1);
    // The log, corrupted under its key: the next boot reads it Unreadable.
    let key = format!("tacsim:claimlog:{AHUNA}");
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let mut found = false;
    for data in rig.store.values_mut() {
        for (k, value) in &mut data.kv {
            if k.as_str() == key {
                *value = json!({ "schema": 99, "site": AHUNA });
                found = true;
            }
        }
    }
    assert!(found, "the clerk wrote the log");
    let mut rig = reboot(rig, &[a]);
    assert!(
        matches!(
            store(&rig.tw).claim_log(&cid(AHUNA)),
            Some(tacsim::surface::claims::ClaimLogState::Unreadable)
        ),
        "the log reads unreadable"
    );
    heard = settle(&mut rig, &[a]);
    for _ in 0..5 {
        let events = fire_landing(&mut rig);
        heard.push_str(&said(&rig.tw, &events, a));
    }
    assert_eq!(heard.matches(&told).count(), 0, "never told again: {heard}");
    assert_eq!(
        lapse_lines(&rig.tw, AHUNA, letter, held.epoch),
        1,
        "the chronicle guards it"
    );
    match store(&rig.tw).claim_log(&cid(AHUNA)) {
        Some(tacsim::surface::claims::ClaimLogState::Present(log)) => {
            assert_eq!(
                log.chronicled.get(&block),
                Some(&held.epoch),
                "the row is back"
            );
        }
        other => panic!("the clerk's next write replaces the log: {other:?}"),
    }
    let mut rig = reboot(rig, &[a]);
    let heard = settle(&mut rig, &[a]);
    assert_eq!(
        heard.matches(&told).count(),
        0,
        "nor after a reboot: {heard}"
    );
    assert_eq!(lapse_lines(&rig.tw, AHUNA, letter, held.epoch), 1);
}

/// **C21 — a pilot holding exactly the fee stakes** (reviewer F1): one minor short, `go` answers
/// `CLAIM_FUNDS` verbatim and offers, orders, holds and queues nothing; with available funds of
/// exactly 4,000.00cr the stake lands, the fee settles and her available balance is 0.
#[test]
fn sm10_a_pilot_holding_exactly_the_fee_stakes() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    // One minor short of the fee: `CLAIM_FUNDS` verbatim, and nothing offered, ordered or queued.
    let excess = available(&rig.tw, a) - 399_999;
    let credit = credit_of(&rig.tw, a);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay #{credit}=#{treasury} {excess}"),
    );
    assert!(out.contains("Paid: transaction #"), "{out}");
    assert_eq!(available(&rig.tw, a), 399_999);
    let contracts = |tw: &TestWorld| {
        tw.world()
            .module_state::<EconomyStore>()
            .expect("economy")
            .snapshot()
            .contracts()
            .count()
    };
    let before = contracts(&rig.tw);
    let (out, queued) = claim(&mut rig, a, "+claim/stake go");
    assert_eq!(
        out.trim(),
        cl::CLAIM_FUNDS
            .replace("{total}", "4000.00cr")
            .replace("{available}", "3999.99cr")
    );
    assert!(queued.is_empty(), "nothing queued");
    assert_eq!(contracts(&rig.tw), before, "no contract offered");
    assert_eq!(
        cl::claim_order_for_tests(rig.tw.world(), a),
        None,
        "no order"
    );
    assert_eq!(available(&rig.tw, a), 399_999, "nothing held");
    // Exactly the fee.
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay #{treasury}=#{credit} 1"),
    );
    assert!(out.contains("Paid: transaction #"), "{out}");
    assert_eq!(available(&rig.tw, a), 400_000);
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    let _ = settle(&mut rig, &[a]);
    for _ in 0..30 {
        if matches!(contract_state(&rig.tw, fee), ContractState::Settled { .. }) {
            break;
        }
        sweep(&mut rig.tw, 60);
    }
    assert!(matches!(
        contract_state(&rig.tw, fee),
        ContractState::Settled { .. }
    ));
    assert_eq!(available(&rig.tw, a), 0);
}

/// **C22 — the seam offers nothing once the order drifts** (reviewer F1/F8): a fee contract
/// cancelled before landing, and a plan whose core moved (another pilot's stake landing first),
/// each leave the order unoffered and file nothing; a mining cycle landing between a renewal's
/// command and its landing does not break it (worked is view, "floor met" is core).
#[test]
fn sm10_the_seam_offers_nothing_once_the_order_drifts() {
    let mut rig = world_on(Some(ALL_RICH));
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    // 1. A fee cancelled before landing.
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let (_, queued) = claim(&mut rig, a, "+claim/stake go");
    let fee = cl::claim_order_for_tests(rig.tw.world(), a).expect("the order");
    let _ = run(&mut rig.tw, a, &format!("@contract/cancel #{fee}"));
    let refused = land(&mut rig, &queued[0]).expect_err("no offer");
    assert!(refused.contains("not currently offered"), "{refused}");
    assert!(block_record(&mut rig.tw, AHUNA, 'C').is_none());
    // 2. Another stake lands first: the core moved.
    let _ = scanned_in(&mut rig, AHUNA, b, tb, 'C');
    let (_, a_queued) = claim(&mut rig, a, "+claim/stake go");
    let (_, b_queued) = claim(&mut rig, b, "+claim/stake go");
    land_all(&mut rig, &b_queued);
    let refused = land(&mut rig, &a_queued[0]).expect_err("no offer");
    assert!(refused.contains("not currently offered"), "{refused}");
    assert_eq!(
        block_record(&mut rig.tw, AHUNA, 'C').and_then(|record| record.claim.map(|c| c.holder)),
        Some(mush_org::PrincipalId::Object(b))
    );
    let _ = settle(&mut rig, &[a, b]);
    // 3. A renewal survives a mining cycle landing in between.
    let _ = stake(&mut rig, AHUNA, a, ta, 'D');
    let _ = work(&mut rig, AHUNA, a, ta, 'D', 50);
    // A stand in D the rig has not worked (the last), sounded, so the cycle is sure to move ore.
    let (stand, _) = *stands_in(&mut rig.tw, AHUNA, 'D')
        .last()
        .expect("a stand in D");
    to_cell_on(&mut rig, AHUNA, ta, stand);
    recharge(&mut rig, ta);
    let _ = sounded(&mut rig, a, ta, true);
    recharge(&mut rig, ta);
    let (_, renew) = command(&mut rig, a, "+claim/renew D go");
    assert_eq!(tenure_actors(&renew), vec![a]);
    let drawn = drawn_block(&mut rig.tw, AHUNA, 'D');
    let (out, queued) = command(&mut rig, a, "+tv/1 mine go");
    assert_eq!(queued.len(), 1, "the mine starts: {out}");
    land_all(&mut rig, &queued);
    let run = record_of(&mut rig.tw, ta)
        .and_then(|record| record.run)
        .expect("a run");
    let (_, queued) = advance_to(&mut rig, run.due_ic);
    land_all(&mut rig, &queued);
    assert!(
        drawn_block(&mut rig.tw, AHUNA, 'D') > drawn,
        "a cycle landed between the renewal's command and its landing"
    );
    land(&mut rig, &renew[0]).expect("the renewal lands");
}

// ---------------------------------------------------------------------------
// The fixer's round on SM.10's review: withdrawn fees, the refund's arms, the contest's bars
// ---------------------------------------------------------------------------

/// `secs` real seconds on, one at a time, everything due fired and landed; what `who` heard.
fn step(rig: &mut Rig, secs: i64, who: &[ObjId]) -> String {
    let mut heard = String::new();
    for _ in 0..secs {
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let events = rig.tw.fire_due(t.saturating_mul(1_000));
        for pilot in who {
            let text = said(&rig.tw, &events, *pilot);
            if !text.is_empty() {
                heard.push_str(&text);
                heard.push('\n');
            }
        }
        let queued = drain(&mut rig.tw);
        land_all(rig, &queued);
    }
    heard
}

/// Write `record` into `tacsim.claims` through a typed transaction (staff's hand).
fn put_block(rig: &mut Rig, record: BlockTenure) {
    let family = handles(&rig.tw).families.claims;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("a claims transaction");
        let key = record.key().expect("a block key");
        txn.replace(&family, &key, record).expect("replaced");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
}

/// The epoch the site's claim log records `letter`'s refund for.
fn refunded_epoch(tw: &TestWorld, entry: &str, letter: char) -> Option<u32> {
    match store(tw).claim_log(&cid(entry)) {
        Some(tacsim::surface::claims::ClaimLogState::Present(log)) => {
            log.refunded.get(&letter.to_string()).copied()
        }
        _ => None,
    }
}

/// **A fee amended after landing lapses the claim** (the fixer's round, correctness F1;
/// D19): Ada's stake lands and, before the clerk fires, she amends its fee contract back to a
/// draft (`@contract/amend`; her own escrow stands); the clerk never accepts it and lapses the
/// claim unpaid (epoch + 1, the cooling row, `UNPAID_LAPSED`), and the Registry's account never
/// moves; at the fee's deadline her escrow comes back.
#[test]
fn sm10_a_fee_amended_after_landing_lapses_the_claim() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let (before, registry) = (available(&rig.tw, a), registry_balance(&rig.tw));
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    let out = run(&mut rig.tw, a, &format!("@contract/amend #{fee}"));
    assert_eq!(contract_state(&rig.tw, fee), ContractState::Draft, "{out}");
    let heard = settle(&mut rig, &[a]);
    assert!(
        heard.contains(
            &cl::UNPAID_LAPSED
                .replace("{block}", &block)
                .replace("{site}", "Ahuna Flank")
        ),
        "{heard}"
    );
    assert_eq!(
        contract_state(&rig.tw, fee),
        ContractState::Draft,
        "never accepted"
    );
    let record = block_record(&mut rig.tw, AHUNA, letter).expect("the record");
    assert!(record.claim.and_then(|claim| claim.lapsed_ic).is_some());
    let tenure = tenure_of(&mut rig.tw, AHUNA, letter);
    assert_eq!(tenure.epoch(), 1);
    assert_eq!(tenure.cooling().len(), 1);
    assert_eq!(
        registry_balance(&rig.tw),
        registry,
        "the Registry took nothing"
    );
    for _ in 0..70 {
        sweep(&mut rig.tw, 60);
    }
    assert_eq!(available(&rig.tw, a), before, "her escrow came back");
    assert_eq!(registry_balance(&rig.tw), registry);
}

/// **A fee re-offered on other terms is never accepted** (the fixer's round, correctness F2):
/// Ada's stake lands; she amends its fee, makes it visible to its parties only (no longer the
/// Public claim fee) and offers it again before the clerk fires; the clerk never accepts it —
/// it reads the fee's terms, not only its state and its sum — and lapses the claim unpaid, and
/// the Registry's account never moves.
#[test]
fn sm10_a_fee_reoffered_on_other_terms_is_never_accepted() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    launched(&mut rig, a, ta);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = scanned_in(&mut rig, AHUNA, a, ta, letter);
    let registry = registry_balance(&rig.tw);
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    for line in [
        format!("@contract/amend #{fee}"),
        format!("@contract/visibility #{fee}=parties"),
        format!("@contract/offer #{fee}"),
    ] {
        let _ = run(&mut rig.tw, a, &line);
    }
    assert_eq!(
        contract_state(&rig.tw, fee),
        ContractState::Offered,
        "offered again"
    );
    let heard = settle(&mut rig, &[a]);
    assert!(
        heard.contains(
            &cl::UNPAID_LAPSED
                .replace("{block}", &block)
                .replace("{site}", "Ahuna Flank")
        ),
        "{heard}"
    );
    assert_eq!(
        contract_state(&rig.tw, fee),
        ContractState::Offered,
        "the Registry never accepted it"
    );
    assert!(
        block_record(&mut rig.tw, AHUNA, letter)
            .and_then(|record| record.claim)
            .and_then(|claim| claim.lapsed_ic)
            .is_some()
    );
    assert_eq!(
        registry_balance(&rig.tw),
        registry,
        "the Registry took nothing"
    );
}

/// **A refund waits for the loser's fee, a second contest waits for the refund, and a refund
/// nobody paid for is never minted** (§9.4, reviewer F2; the fixer's round, tests T4 and
/// correctness F3). Block one: Bryn and Cato each sound it loudly (Cato first), Ada stakes it
/// and the clerk accepts her fee; Bryn contests at once. While Ada's fee is accepted and not yet
/// settled, no refund is posted; right after the clerk accepts Bryn's fee, Cato's contest
/// answers `CLAIM_PENDING` (the refund still owed) and escrows nothing; once Ada's fee settles
/// the clerk refunds her 2,000 once, the log records it, and Cato's contest previews. Block two:
/// Ada stakes it, Bryn contests it, and staff's hand points the owed refund at a fee Ada
/// cancelled — the clerk tells Ada `CONTEST_LOST_UNPAID`, records the refund done, and posts
/// nothing: Ada's account and the Registry's move only by Bryn's fee.
#[test]
#[allow(clippy::too_many_lines)]
fn sm10_a_refund_waits_for_the_losers_fee_and_never_mints() {
    let mut rig = world_on(Some(ALL_RICH));
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    let (c, _, tc) = on_ahuna(&mut rig, "Cato", "CatoAcct", "Tern");
    for (who, tv) in [(a, ta), (b, tb), (c, tc)] {
        launched(&mut rig, who, tv);
    }
    let stands = stands_in(&mut rig.tw, AHUNA, 'C');
    to_cell_on(&mut rig, AHUNA, tc, stands[0].0);
    let _ = sounded(&mut rig, c, tc, false);
    to_cell_on(&mut rig, AHUNA, tb, stands[1].0);
    let _ = sounded(&mut rig, b, tb, false);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let _ = claim_go(&mut rig, a, "+claim/stake go");
    let a_fee = block_record(&mut rig.tw, AHUNA, 'C')
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("Ada's fee");
    for _ in 0..30 {
        if contract_state(&rig.tw, a_fee) != ContractState::Offered {
            break;
        }
        let _ = step(&mut rig, 1, &[a]);
    }
    let unsettled = |tw: &TestWorld, id: u64| {
        matches!(
            contract_state(tw, id),
            ContractState::Active | ContractState::ReadyToSettle
        )
    };
    assert!(unsettled(&rig.tw, a_fee), "accepted, not yet settled");
    let ada_before = available(&rig.tw, a);
    let _ = claim_go(&mut rig, b, "+claim/contest C go");
    let b_fee = block_record(&mut rig.tw, AHUNA, 'C')
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("Bryn's fee");
    for _ in 0..30 {
        if contract_state(&rig.tw, b_fee) != ContractState::Offered {
            break;
        }
        let _ = step(&mut rig, 1, &[a, b]);
    }
    // Right after the clerk accepts Bryn's fee the refund is owed: a second contest waits.
    let cato_before = available(&rig.tw, c);
    let (out, queued) = claim(&mut rig, c, "+claim/contest C go");
    assert_eq!(
        out.trim(),
        cl::CLAIM_PENDING
            .replace("{block}", "C")
            .replace("{site}", "Ahuna Flank"),
        "the owed refund holds the block"
    );
    assert!(queued.is_empty());
    assert_eq!(available(&rig.tw, c), cato_before, "nothing escrowed");
    // While Ada's fee is unsettled, nothing is refunded.
    let mut waited = false;
    for _ in 0..120 {
        if !unsettled(&rig.tw, a_fee) {
            break;
        }
        waited = true;
        assert_eq!(
            available(&rig.tw, a),
            ada_before,
            "no refund before the loser's fee settles"
        );
        assert_eq!(refunded_epoch(&rig.tw, AHUNA, 'C'), None);
        let _ = step(&mut rig, 1, &[a]);
    }
    assert!(waited, "the refund met the loser's fee unsettled");
    assert!(matches!(
        contract_state(&rig.tw, a_fee),
        ContractState::Settled { .. }
    ));
    let _ = settle(&mut rig, &[a, b]);
    assert_eq!(
        available(&rig.tw, a),
        ada_before + 200_000,
        "her term fee came back once"
    );
    let epoch = block_record(&mut rig.tw, AHUNA, 'C')
        .and_then(|record| record.claim)
        .map(|claim| claim.epoch)
        .expect("Bryn's claim");
    assert_eq!(refunded_epoch(&rig.tw, AHUNA, 'C'), Some(epoch));
    let (preview, queued) = claim(&mut rig, c, "+claim/contest C");
    assert!(
        preview.starts_with("Block C at Ahuna Flank: the site heard your sounding at "),
        "the refund paid, the block is contestable again: {preview}"
    );
    assert!(queued.is_empty());
    // Block two: a refund owed against a fee nobody paid is never minted.
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'M');
    let (_, dropped) = claim(&mut rig, a, "+claim/stake go");
    let cancelled = cl::claim_order_for_tests(rig.tw.world(), a).expect("the order");
    let _ = run(&mut rig.tw, a, &format!("@contract/cancel #{cancelled}"));
    assert_eq!(contract_state(&rig.tw, cancelled), ContractState::Cancelled);
    assert!(land(&mut rig, &dropped[0]).is_err(), "never offered");
    let stands = stands_in(&mut rig.tw, AHUNA, 'P');
    to_cell_on(&mut rig, AHUNA, tb, stands[0].0);
    let _ = sounded(&mut rig, b, tb, false);
    let held = stake(&mut rig, AHUNA, a, ta, 'P');
    for _ in 0..30 {
        if matches!(
            contract_state(&rig.tw, held.fee.expect("a fee")),
            ContractState::Settled { .. }
        ) {
            break;
        }
        sweep(&mut rig.tw, 60);
    }
    let (ada, registry) = (available(&rig.tw, a), registry_balance(&rig.tw));
    let _ = claim_go(&mut rig, b, "+claim/contest P go");
    let mut record = block_record(&mut rig.tw, AHUNA, 'P').expect("P's record");
    let won = record.claim.as_mut().expect("Bryn's claim");
    let owed = won.refund.as_mut().expect("the refund owed Ada");
    owed.paid_by = cancelled;
    let (d_fee, d_epoch) = (won.fee.expect("Bryn's fee"), won.epoch);
    put_block(&mut rig, record);
    let heard = settle(&mut rig, &[a, b]);
    assert!(
        heard.contains(
            &cl::CONTEST_LOST_UNPAID
                .replace("{contester}", "Bryn")
                .replace("{block}", "P")
                .replace("{site}", "Ahuna Flank")
        ),
        "{heard}"
    );
    for _ in 0..30 {
        if matches!(
            contract_state(&rig.tw, d_fee),
            ContractState::Settled { .. }
        ) {
            break;
        }
        sweep(&mut rig.tw, 60);
    }
    let _ = settle(&mut rig, &[a, b]);
    assert_eq!(
        refunded_epoch(&rig.tw, AHUNA, 'P'),
        Some(d_epoch),
        "recorded done"
    );
    assert_eq!(available(&rig.tw, a), ada, "nothing refunded");
    assert_eq!(
        registry_balance(&rig.tw),
        registry + 400_000,
        "the Registry moved by Bryn's fee alone"
    );
}

/// **A cooling challenger is refused before anything is escrowed** (the fixer's round,
/// correctness F4; lead ruling 5, OC15): Ada stakes and releases a block; after the lapse Ada
/// and Cass (who controls Ada) each sound it loudly; Bryn reads it later and takes it over. Ada's
/// and Cass's contests answer `STAKE_COOLING`, queue nothing and hold nothing; Sol, a stranger
/// whose loud sounding came between, contests it.
#[test]
fn sm10_a_cooling_challenger_is_refused_before_any_escrow() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    let (cass, _, tcass) = on_ahuna(&mut rig, "Cass", "CassAcct", "Tern");
    let (s, _, ts) = on_ahuna(&mut rig, "Sol", "SolAcct", "Kite");
    for (who, tv) in [(a, ta), (b, tb), (cass, tcass), (s, ts)] {
        launched(&mut rig, who, tv);
    }
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let _ = stake(&mut rig, AHUNA, a, ta, letter);
    let _ = claim_go(&mut rig, a, &format!("+claim/release {letter} go"));
    let _ = settle(&mut rig, &[a]);
    if let Some(obj) = rig.tw.world_mut().get_mut(a) {
        obj.owner = cass;
    }
    let stands = stands_in(&mut rig.tw, AHUNA, letter);
    for (i, (who, tv)) in [(a, ta), (cass, tcass), (s, ts)].into_iter().enumerate() {
        to_cell_on(&mut rig, AHUNA, tv, stands[i].0);
        let _ = sounded(&mut rig, who, tv, false);
    }
    let _ = stake(&mut rig, AHUNA, b, tb, letter);
    let until = tenure_of(&mut rig.tw, AHUNA, letter).cooling()[0].until.at;
    let cooling = cl::STAKE_COOLING
        .replace("{block}", &block)
        .replace("{until}", &tacsim::port::licence::ic_stamp(until));
    for who in [a, cass] {
        let before = available(&rig.tw, who);
        let (out, queued) = claim(&mut rig, who, &format!("+claim/contest {letter} go"));
        assert_eq!(out.trim(), cooling, "#{} cools", who.0);
        assert!(queued.is_empty());
        assert_eq!(available(&rig.tw, who), before, "nothing escrowed");
    }
    let out = claim_go(&mut rig, s, &format!("+claim/contest {letter} go"));
    assert!(out.contains("You contest block"), "{out}");
}

/// **A contest withdrawn after it lands evicts nobody for free unheard** (the fixer's round,
/// correctness F5): Bryn's contest of Ada's block lands and he cancels its fee before the clerk
/// fires; the clerk lapses the block unpaid (Bryn cools, `UNPAID_LAPSED`), tells Ada
/// `CONTEST_LOST`, and returns her paid term fee once; the Registry's account moves only by
/// that refund.
#[test]
fn sm10_a_contest_withdrawn_after_landing_tells_the_loser() {
    let mut rig = world_on(None);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let letter = ore_block(&mut rig.tw, AHUNA);
    let block = letter.to_string();
    let cell = stand_in(&mut rig.tw, AHUNA, letter);
    to_cell_on(&mut rig, AHUNA, tb, cell);
    let _ = sounded(&mut rig, b, tb, false);
    let held = stake(&mut rig, AHUNA, a, ta, letter);
    let a_fee = held.fee.expect("Ada's fee");
    for _ in 0..30 {
        if matches!(
            contract_state(&rig.tw, a_fee),
            ContractState::Settled { .. }
        ) {
            break;
        }
        sweep(&mut rig.tw, 60);
    }
    let (ada, registry) = (available(&rig.tw, a), registry_balance(&rig.tw));
    let _ = claim_go(&mut rig, b, &format!("+claim/contest {letter} go"));
    let b_fee = block_record(&mut rig.tw, AHUNA, letter)
        .and_then(|record| record.claim)
        .and_then(|claim| claim.fee)
        .expect("Bryn's fee");
    let _ = run(&mut rig.tw, b, &format!("@contract/cancel #{b_fee}"));
    assert_eq!(contract_state(&rig.tw, b_fee), ContractState::Cancelled);
    let heard = settle(&mut rig, &[a, b]);
    assert!(
        heard.contains(
            &cl::CONTEST_LOST
                .replace("{contester}", "Bryn")
                .replace("{block}", &block)
                .replace("{site}", "Ahuna Flank")
        ),
        "the loser is told: {heard}"
    );
    assert!(
        heard.contains(
            &cl::UNPAID_LAPSED
                .replace("{block}", &block)
                .replace("{site}", "Ahuna Flank")
        ),
        "{heard}"
    );
    let tenure = tenure_of(&mut rig.tw, AHUNA, letter);
    assert!(
        tenure
            .cooling()
            .iter()
            .any(|row| row.holder == mush_org::PrincipalId::Object(b)),
        "the withdrawn contester cools"
    );
    assert_eq!(
        available(&rig.tw, a),
        ada + 200_000,
        "her term fee came back"
    );
    assert_eq!(
        registry_balance(&rig.tw),
        registry - 200_000,
        "the Registry returned the term it no longer gives her, and took nothing from Bryn"
    );
}

// ---------------------------------------------------------------------------
// C23 — a block with no deposits
// ---------------------------------------------------------------------------

/// C23's sequence on `table`: the lines it printed and heard.
fn c23_world(table: &'static str) -> Vec<String> {
    let mut rig = world_with_table(table);
    let (a, _, ta) = on_ahuna(&mut rig, "Ada", "AdaAcct", "Salt Moth");
    let (b, _, tb) = on_ahuna(&mut rig, "Bryn", "BrynAcct", "Grey Heron");
    // Doc 20 cut SM.11: aligned before the launches, so the jump to the aligned instant never
    // leaves a rig a day idle on the ground (the idle return would bring it home).
    align(&mut rig);
    launched(&mut rig, a, ta);
    launched(&mut rig, b, tb);
    let mut lines = Vec::new();
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    lines.push(claim_lines(&claim_go(&mut rig, a, "+claim/stake go")));
    lines.push(claim_lines(&settle(&mut rig, &[a])));
    if table == ALL_BARREN {
        let record = block_record(&mut rig.tw, AHUNA, 'C').expect("the record");
        assert!(record.claim.is_some(), "the record alone");
        assert!(
            opened(&mut rig.tw, AHUNA, 'C').is_empty(),
            "no industry claim to file"
        );
    }
    let _ = scanned_in(&mut rig, AHUNA, b, tb, 'C');
    let (out, queued) = claim(&mut rig, b, "+claim/stake go");
    assert!(queued.is_empty());
    lines.push(out.trim().to_owned());
    recharge(&mut rig, tb);
    let (out, queued) = command(&mut rig, b, "+tv/1 mine go");
    assert!(queued.is_empty(), "a stranger's mine is refused");
    lines.push(out.trim().to_owned());
    let frame = map_frame(&mut rig, b, json!({ "view": "site" }));
    lines.push(format!("{:?}", claim_cells(&frame)));
    lines.push(run(&mut rig.tw, b, "+registry ahuna flank"));
    let term_end = block_record(&mut rig.tw, AHUNA, 'C')
        .and_then(|record| record.claim)
        .map(|claim| claim.term_end_ic)
        .expect("the claim");
    let events = to_ic(&mut rig, term_end + 60);
    lines.push(claim_lines(&said(&rig.tw, &events, a)));
    lines.push(claim_lines(&settle(&mut rig, &[a])));
    let tenure = tenure_of(&mut rig.tw, AHUNA, 'C');
    assert_eq!(tenure.epoch(), 1, "the place lapsed");
    assert_eq!(tenure.cooling().len(), 1, "the holder cools");
    assert_eq!(lapse_lines(&rig.tw, AHUNA, 'C', 0), 1, "one canon line");
    relicense(&mut rig, a);
    let _ = scanned_in(&mut rig, AHUNA, a, ta, 'C');
    let (out, queued) = claim(&mut rig, a, "+claim/stake go");
    assert!(queued.is_empty());
    lines.push(out.trim().to_owned());
    lines
}

/// **C23 — a block with no deposits is claimed like any other** (reviewer F4): in the all-barren
/// world a stake lands and writes the block record with no industry claim; a second staker is
/// refused "That block is under claim."; a stranger's mine is refused the same; `#` is drawn and
/// `+registry` lists it; the clerk lapses it at its term's end (epoch + 1 and the cooling row in
/// industry's tenure, `LAPSED_TOLD`, one canon line); the holder is then refused
/// `STAKE_COOLING` — and every line is byte-identical to the same sequence in the all-rich world.
#[test]
fn sm10_a_block_with_no_deposits_is_claimed_like_any_other() {
    let barren = c23_world(ALL_BARREN);
    println!("C23: {barren:#?}");
    assert_eq!(barren[2], "That block is under claim.");
    assert!(
        barren[3].contains("That block is under claim."),
        "{}",
        barren[3]
    );
    assert!(barren[4] != "[]", "the edge is drawn");
    assert!(
        barren[5].contains("  Block C: Ada; term to "),
        "{}",
        barren[5]
    );
    assert!(
        barren[7].contains("Your claim on block C at Ahuna Flank has lapsed"),
        "{}",
        barren[7]
    );
    assert!(
        barren[8].starts_with("Block C's last claim lapsed from you"),
        "{}",
        barren[8]
    );
    assert_eq!(
        barren,
        c23_world(ALL_RICH),
        "byte-identical to the all-rich world"
    );
}
