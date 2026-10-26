//! **Doc 18 cut IC6 acceptance — the Co-op programme and boarding** (doc 18
//! §4.1, §4.2 "The airlock and the berth" and "Compartments", §4.7; D-18-6,
//! D-18-21, D-18-23, D-18-24, D-18-25, D-18-34, D-18-39; 18a §1.3 C01/C05 and
//! §3; lead rulings L1–L21 of the IC6 brief).
//!
//! A pilot at the Registry & Co-op Desk types `+coop/enrol Salt Moth` and, in
//! one command and one batch, becomes a First Captain: an account opened by
//! Ceres Clearing, the Trust's lowest-id free Dray leased (1 minor on the
//! Manual clock), the stake posted by the Trust keyed by the lease, the hold
//! vault on the hull's stowage, the hull renamed with its mark kept, and one
//! enrolment document per gateway account, ever. The operate gate is the live
//! lease: a staff cancel revokes the charter and every verb reads it. The
//! tenant boards from the gallery where the hull is cradled and leaves by the
//! airlock while docked; the hull rooms' `look` names the hull and the port and
//! reads the hold's pools at look time.
//!
//! Composed on the composition root (`server_modules_with_tacsim` and the
//! wired test-world layer; `Roster::bare`, no sheet, so a spawned player types
//! and walks). No test here asserts a passage duration (D-18-50).

use std::collections::{BTreeMap, HashMap};

use crate::vertex;
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, CanonicalId, GOD, LIMBO,
    ModuleWrite, ObjId,
};
use mush_economy::{
    ContractId, ContractState, CustodyRef, Deliverable, EconomicClock, EconomyConfig, EconomyStore,
    Encumbrance, EscalationSubject, ObligationCause, ObligationStatus,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_org::{OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::coop::{
    BAD_NAME, BOOKS_UNREADABLE, CHARTER_NOT_IN_FORCE, ENROLLED_ONCE, EnrolmentState,
    EnrolmentStatus, FAILED, NAME_TAKEN, NAME_USAGE, NO_ACCOUNT_MATCH, NO_CAPITAL, NO_HULL,
    NOT_OPEN, REGISTRAR_LINE, enrol_gate, enrolment_key, stake_key,
};
use tacsim::port::{HullRow, PortDef, PortMap, PortMapState, readiness, room_key};
use tacsim::port_tick::pending_notices;
use tacsim::{TacsimConfig, TacsimStore};

// ---------------------------------------------------------------------------
// The rig (hulls.rs's, one file-local copy)
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

/// How a test roster is composed.
#[derive(Clone)]
struct Roster {
    /// The test world's layer (`tacsim_test_world_module_wired`), or the
    /// production one (`tacsim_module_wired`).
    test_world: bool,
    /// `[tacsim].enabled`.
    enabled: bool,
    /// Compose the test-only [`Latch`] (the whole-economy freeze seam).
    freezable: bool,
}

impl Roster {
    /// No sheet, the test world's layer.
    fn bare() -> Roster {
        Roster {
            test_world: true,
            enabled: true,
            freezable: false,
        }
    }

    /// No sheet, the production layer (the rate refusal).
    fn bare_real() -> Roster {
        Roster {
            test_world: false,
            enabled: true,
            freezable: false,
        }
    }

    /// The same roster with the layer switched off by config.
    fn disabled(mut self) -> Roster {
        self.enabled = false;
        self
    }

    /// The same roster with the test-only freeze seam.
    fn freezable(mut self) -> Roster {
        self.freezable = true;
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
        let cfg = TacsimConfig {
            enabled: self.enabled,
            ..TacsimConfig::default()
        };
        let (mut mods, inventory) = comp::server_modules_with_tacsim(
            mush_weather::WeatherConfig::default(),
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            comp::recovery_policy(&cfg.recovery),
        );
        mods.push(comp::worldgen_module_with(
            Arc::clone(&llm),
            mush_worldgen::WorldgenConfig::default(),
        ));
        mods.push(comp::npc_module_with(
            comp::npc_agent_adapter(),
            &mush_config::config::NpcCfg::default(),
        ));
        if self.test_world {
            mods.push(comp::tacsim_test_world_module_wired(cfg, true, inventory));
        } else {
            mods.push(comp::tacsim_module_wired(cfg, true, inventory));
        }
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
        if self.freezable {
            mods.push(Box::new(Latch));
        }
        mods
    }
}

/// A test-only module carrying the seam `e4c` and `e7c9` use for the same job:
/// the whole-economy latch has a staff CLEAR (`@econ/unfreeze`) and no staff
/// set, so this drives it the way boot's recovery pass does. It impersonates no
/// production module.
#[derive(Default)]
struct Latch;

impl mush_core::GameModule for Latch {
    fn name(&self) -> &'static str {
        "tacsim_test_latch"
    }
    fn register(
        &mut self,
        reg: &mut mush_core::module::Registrar<'_>,
    ) -> Result<(), mush_core::module::ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            mush_core::CommandSpec {
                name: "@econfreeze".into(),
                shape: mush_core::ArgShape::Raw,
                help: "test: set the whole-economy latch".into(),
                ..mush_core::CommandSpec::default()
            },
            Arc::new(
                |ctx: &mut mush_core::Ctx<'_>| -> Result<(), mush_core::CmdError> {
                    mush_economy::set_economy_frozen(ctx, true);
                    Ok(())
                },
            ),
        )?;
        Ok(())
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

/// A booted world and its oracle.
struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    roster: Roster,
    /// The cumulative store: every module write since the first boot, folded
    /// the way the store folds them.
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    /// The typed-state documents the seeded pools wrote (`(ns, key)` → value),
    /// hydrated into the engine after a reboot as the server's boot does.
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

/// Boot `roster` without settling the capital.
fn boot_unsettled(roster: Roster) -> Rig {
    let mut tw = TestWorld::with_modules(roster.modules());
    with_offer_keys(&mut tw);
    no_refused_writes(&tw);
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

/// One economy sweep `secs` on (`ports.rs`'s).
fn sweep(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

/// Boot `roster` and sweep once so the capital settles; the desk is open.
fn boot(roster: Roster) -> Rig {
    let mut rig = boot_unsettled(roster);
    sweep(&mut rig.tw, 120);
    assert!(
        enrol_gate(&readiness(rig.tw.world())).is_ok(),
        "the desk is not open after the first sweep:\n{}",
        readiness(rig.tw.world()).render().join("\n")
    );
    // Doc 18 cut IC8: the gate also reads the ports' stock (Pools, Listings and
    // MarketFacts, `readiness_in`), so the port tick is fired and landed until the
    // stock clerks have seeded and listed every ask.
    ensure_stocked(&mut rig);
    rig
}

/// Fold every write since boot and reboot `rig` into its roster (or
/// `roster`), re-installing the oracle.
fn reboot_with(
    rig: Rig,
    roster: Roster,
    edit: impl FnOnce(&mut HashMap<SmolStr, mush_core::ModuleBootData>),
) -> Rig {
    let Rig {
        mut tw,
        oracle,
        mut store,
        typed,
        ..
    } = rig;
    fold_module_ops_into(&mut store, tw.take_module_ops());
    edit(&mut store);
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
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy state")
}

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&lane_id(), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn ceres_map(tw: &TestWorld) -> PortMap {
    port_map(tw, "ceres_port")
}

fn port_def(entry: &str) -> PortDef {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    }
}

/// A station room by its pack-local name, in `entry`'s area.
fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = port_def(entry);
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn c01(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c01_berth_gallery")
}

fn c03(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c03_arrivals")
}

fn c05(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c05_registry")
}

fn p01(tw: &TestWorld) -> ObjId {
    station_room(tw, "stickney_port", "room.p01_berth_gallery")
}

fn hull_rooms(row: &HullRow) -> BTreeMap<&str, ObjId> {
    row.rooms
        .iter()
        .map(|(id, obj)| (id.as_str(), ObjId(*obj)))
        .collect()
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn name(tw: &TestWorld, obj: ObjId) -> String {
    tw.world()
        .get(obj)
        .map(|o| o.name.clone())
        .expect("an object")
}

fn attr_text(tw: &TestWorld, obj: ObjId, attr: &str) -> Option<String> {
    tw.world()
        .attr(obj, attr)
        .and_then(|a| a.as_text().map(str::to_owned))
}

/// A gateway account.
fn account(name: &str, seed: i64) -> AccountId {
    AccountId::mint(name, seed).expect("an account id")
}

/// `@teleport` (as GOD) `who` to `room`.
fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let _ = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(location(tw, who), Some(room), "the teleport landed");
}

/// A pilot on `account`, standing at the Registry & Co-op Desk.
fn pilot_on(rig: &mut Rig, who: &str, account: AccountId) -> ObjId {
    let pilot = rig.tw.spawn_player(who);
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(pilot, account);
    }
    let desk = c05(&rig.tw);
    teleport(&mut rig.tw, pilot, desk);
    pilot
}

/// A pilot on an account of their own (`{who}Acct`, `seed`).
fn pilot(rig: &mut Rig, who: &str, seed: i64) -> ObjId {
    pilot_on(rig, who, account(&format!("{who}Acct"), seed))
}

/// The Trust's programme account.
fn trust_account(tw: &TestWorld) -> mush_economy::AccountId {
    let def = port_def("ceres_port");
    let org = def.programme.expect("a programme").org;
    mush_economy::AccountId(
        ceres_map(tw).orgs[&org]
            .account
            .expect("the Trust has an account"),
    )
}

fn trust(tw: &TestWorld) -> PrincipalId {
    let def = port_def("ceres_port");
    let org = def.programme.expect("a programme").org;
    PrincipalId::Organization(OrgId(ceres_map(tw).orgs[&org].org))
}

fn balance(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .posted_balance
        .minor()
}

/// The pilot's own tacsim.credit account.
fn own_account(tw: &TestWorld, pilot: ObjId) -> Option<mush_economy::AccountId> {
    tacsim::port::credit::own_account(tw.world(), pilot).map(|a| a.id)
}

/// What one enrolment wrote, counted.
#[derive(Debug, PartialEq, Eq)]
struct Counts {
    contracts: usize,
    accounts: usize,
    vaults: usize,
    transactions: usize,
    enrolments: usize,
}

fn counts(tw: &TestWorld) -> Counts {
    let snap = economy(tw).snapshot();
    Counts {
        contracts: snap.contracts().count(),
        accounts: snap.accounts().count(),
        vaults: snap.vaults().count(),
        transactions: economy(tw).transactions().count(),
        enrolments: store(tw).enrolments().count(),
    }
}

/// The pilot's live lease.
fn lease_of(tw: &TestWorld, pilot: ObjId) -> ContractId {
    tacsim::port::live_charter(tw.world(), pilot)
        .expect("a live charter")
        .contract
}

/// Enrol `pilot` (at the desk) as `ship`; the reply. (The desk opens only once the
/// ports' stock is seeded and listed — doc 18 cut IC8 — which [`boot`] sees to.)
fn enrol(rig: &mut Rig, pilot: ObjId, ship: &str) -> String {
    run(&mut rig.tw, pilot, &format!("+coop/enrol {ship}"))
}

/// A fresh request/commit token per landing, across every test in the process.
fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0xc0_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Drive one queued submission the way the world task does (`port_tick.rs`'s
/// `land_one`), keeping its typed post-images for a later reboot.
fn land_typed(rig: &mut Rig, submit: &mush_core::AuthoritativeSubmit) -> Result<(), String> {
    use mush_core::{
        CommitToken, IdempotencyKey, IdempotencyScope, OfferExecutionContext, OfferSubmission,
        RequestUid, ScopedIdempotencyKey,
    };
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

/// The commerce lines of `@tacsim/port status` (doc 18 cut IC8): every Pools,
/// Listings and MarketFacts line — and, since cut IC10, the InventoryFacts line —
/// with whether it reads `[ready]`.
fn commerce_ready(tw: &mut TestWorld) -> (bool, String) {
    let out = run(tw, GOD, "@tacsim/port status");
    let gated: Vec<&str> = out
        .lines()
        // Doc 18 cut IC12: the gate is every report line but the sky (D-18-50).
        // doc 20 SM.1: the geology line is reported and never gated
        .filter(|line| {
            line.ends_with(']') && !line.starts_with("sky: ") && !line.starts_with("geology: ")
        })
        .collect();
    let ready = !gated.is_empty() && gated.iter().all(|line| line.ends_with("[ready]"));
    (ready, out)
}

/// **Fire and land the port tick until the gate's commerce lines are ready**
/// (doc 18 cut IC8): the stock clerks mint, dissolve and list, one step a fire.
/// Bounded; panics with the last report.
fn ensure_stocked(rig: &mut Rig) {
    for _ in 0..40 {
        let (ready, _) = commerce_ready(&mut rig.tw);
        if ready {
            return;
        }
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let mut queued = Vec::new();
        while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
            let owner = submit.action.key().owner.as_str();
            if owner == tacsim::NS || owner == "inventory" {
                assert_ne!(submit.actor, GOD, "GOD never submits from the port tick");
                queued.push(*submit);
            }
        }
        for submit in &queued {
            land_typed(rig, submit).expect("a port-tick submission lands");
        }
    }
    let (_, out) = commerce_ready(&mut rig.tw);
    panic!("the ports were not stocked within 40 fires:\n{out}");
}

/// Seed `qty` of `good` into `vault`'s pool through `lot` (`port_tick.rs`'s
/// mint-origin-then-dissolve, the lot a parameter).
fn seed_pool(rig: &mut Rig, vault: mush_economy::VaultId, good: &str, qty: u64, lot: LotId) {
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
        // The typed post-images a server would persist and hydrate at boot.
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

/// The hold vault on a row's stowage.
fn hold_vault(tw: &TestWorld, row: &HullRow) -> mush_economy::VaultId {
    let stowage = hull_rooms(row)["stowage"];
    economy(tw)
        .snapshot()
        .vault_by_backing(&CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id
}

/// The Dray's cargo hold and the goods table, as the capacity function reads them.
fn cargo_and_goods() -> (
    tacsim::port::capacity::Compartment,
    tacsim::definitions::GoodsTable,
) {
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    let pack = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray pack");
    let Some(PackEntry::Unit(spec)) = pack.entry(&cid(tacsim::definitions::content::DRAY)) else {
        panic!("the Dray unit");
    };
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let cargo = cid("cargo");
    let def = logistics.stowage.get(&cargo).expect("the cargo hold");
    (
        tacsim::port::capacity::Compartment::from_stowage(&cargo, def),
        goods,
    )
}

/// The outbound load of lens 7 (doc 18 cut IC8 re-derived it, D-18-39): the
/// metals/ice load at both of the hold's limits, solved from the goods table, never
/// typed — and 53 t propellant, 19 crew-days of provisions (the flip law since doc 19
/// cut A.2; 42 t and 21 crew-days in the slice).
fn outbound() -> (u64, u64) {
    let (cargo, goods) = cargo_and_goods();
    vertex::both_limits_vertex(&goods, &cargo, &cid("metals"), &cid("volatiles"))
        .expect("an integral vertex")
}

fn seed_outbound(rig: &mut Rig, row: &HullRow) {
    let vault = hold_vault(&rig.tw, row);
    let (metals, ice) = outbound();
    seed_pool(rig, vault, "metals", metals, LotId(1));
    seed_pool(rig, vault, "volatiles", ice, LotId(2));
    seed_pool(rig, vault, "propellant", 53, LotId(3));
    seed_pool(rig, vault, "provisions", 19, LotId(4));
}

/// `{t} t · {m³} m³` for `qty` tonnes of `good`.
fn cell(good: &str, qty: u64) -> String {
    let (_, goods) = cargo_and_goods();
    let litres = goods
        .good(&cid(good))
        .and_then(|row| row.measures.get(&cid("si.volume_l")).copied())
        .expect("a packed volume");
    format!(
        "{qty} t · {} m³",
        tacsim::port::capacity::format_m3(qty * litres)
    )
}

/// The hold's load cells over the outbound load, `indent` before each line.
fn load_cells(indent: &str) -> Vec<String> {
    let (metals, ice) = outbound();
    vec![
        format!("{indent}metal concentrate: {}", cell("metals", metals)),
        format!("{indent}water ice: {}", cell("volatiles", ice)),
        format!("{indent}structural alloy stock: 0 t · 0.0 m³"),
        format!("{indent}yard fittings, crated: 0 t · 0.0 m³"),
        // Doc 19 cut A.8: the moon goods, two more zero cells.
        format!("{indent}instrument boards, crated: 0 t · 0.0 m³"),
        format!("{indent}carbon billet, pressed: 0 t · 0.0 m³"),
        format!("{indent}TOTAL: 100 t · 120.0 m³"),
        format!("{indent}LIMIT: 100 t · 120 m³"),
        format!("{indent}FULL: mass and packed volume"),
    ]
}

/// The Cargo Hold's gauge block.
fn load_cells_block() -> Vec<String> {
    let mut out = vec!["Load cells:".to_owned()];
    out.extend(load_cells("  "));
    out
}

/// The passage's and the bridge's summary body.
fn summary_body() -> Vec<String> {
    let (metals, ice) = outbound();
    vec![
        format!(
            "  HOLD {metals} t metal concentrate, {ice} t water ice; 100/100 t · 120.0/120 m³ · \
             FULL: mass and packed volume"
        ),
        "  TANKAGE 53/80 t".to_owned(),
        "  STORES 19/120 crew-days".to_owned(),
    ]
}

/// `out` contains `block`, line for line, contiguously.
fn has_block(out: &str, block: &[&str]) -> bool {
    out.contains(&block.join("\n"))
}

/// A fresh rig with one enrolled pilot (`Pilot`, *Salt Moth*, FPC-D-0401).
fn enrolled() -> (Rig, ObjId) {
    let mut rig = boot(Roster::bare());
    let who = pilot(&mut rig, "Pilot", 1);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains("now the Salt Moth"), "{out}");
    (rig, who)
}

// ---------------------------------------------------------------------------
// 1. Enrol once
// ---------------------------------------------------------------------------

#[test]
fn ic6_enrol_once_opens_the_account_charters_stakes_and_names_the_hull() {
    let mut rig = boot(Roster::bare());
    let trust_acct = trust_account(&rig.tw);
    let trust_before = balance(&rig.tw, trust_acct);
    assert_eq!(trust_before, 25_000_000, "the Trust holds its 250,000.00cr");
    let who = pilot(&mut rig, "Pilot", 1);
    assert_eq!(own_account(&rig.tw, who), None);
    let out = enrol(&mut rig, who, "Salt Moth");
    let acct = own_account(&rig.tw, who).expect("an account was opened");
    for line in [
        format!("Ceres Clearing opens account #{} in your name.", acct.0),
        "The Co-op charters you FPC-D-0401, laid up as the Slack Water, now the Salt Moth, \
         cradled at the Visiting Berth Gallery."
            .to_owned(),
        "25000.00cr staked to you. Buyout 90000.00cr. Ilse Oduya-Brandt countersigns.".to_owned(),
    ] {
        assert!(out.contains(&line), "missing {line:?} in:\n{out}");
    }
    // +credit.
    let credit = run(&mut rig.tw, who, "+credit");
    assert!(
        credit.contains("Credit: 25000.00cr posted, 0.00cr held, 25000.00cr available."),
        "{credit}"
    );
    // The lease.
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let hull = ObjId(row.hull);
    let lease = lease_of(&rig.tw, who);
    let trust = trust(&rig.tw);
    let contract = economy(&rig.tw)
        .snapshot()
        .contract(lease)
        .expect("the lease")
        .clone();
    assert_eq!(contract.state, ContractState::Active);
    assert_eq!(contract.issuer, trust);
    assert_eq!(
        contract.consent.parties,
        vec![trust.clone(), PrincipalId::Object(who)]
    );
    let deliverables = &contract.consent.terms.deliverables;
    assert_eq!(deliverables.len(), 2);
    assert!(matches!(
        &deliverables[0],
        Deliverable::Collateral { object, from } if *object == hull && *from == trust
    ));
    let Deliverable::Obligation { template } = &deliverables[1] else {
        panic!("the second deliverable is the rent: {deliverables:?}");
    };
    assert_eq!(template.amount.minor(), 1);
    assert_eq!(template.schedule.clock, EconomicClock::Manual);
    assert_eq!(template.schedule.next_due, i64::MAX);
    let rent = economy(&rig.tw)
        .snapshot()
        .obligations()
        .find(|o| o.cause == ObligationCause::Contract(lease))
        .expect("the rent obligation")
        .clone();
    assert_eq!(rent.status, ObligationStatus::Active);
    assert_eq!(rent.amount.minor(), 1);
    // The hull's asset carries the Lease.
    let asset = economy(&rig.tw)
        .snapshot()
        .asset(hull)
        .expect("the hull's asset")
        .clone();
    assert_eq!(asset.title_owner, trust, "the title stays the Trust's");
    assert!(
        asset
            .encumbrances
            .iter()
            .any(|e| matches!(e, Encumbrance::Lease { contract, .. } if *contract == lease)),
        "{:?}",
        asset.encumbrances
    );
    // The stake: one transaction under the lease's key, 2,500,000 minor.
    let key = stake_key(lease);
    assert_eq!(key.namespace, "tacsim.backer.stake");
    assert_eq!(key.key, lease.0.to_string());
    let stakes: Vec<_> = economy(&rig.tw)
        .transactions()
        .filter(|t| t.idempotency == key)
        .collect();
    assert_eq!(stakes.len(), 1, "one stake transaction");
    assert_eq!(balance(&rig.tw, acct), 2_500_000);
    assert_eq!(balance(&rig.tw, trust_acct), trust_before - 2_500_000);
    // The hold vault.
    let stowage = hull_rooms(&row)["stowage"];
    let vault = economy(&rig.tw)
        .snapshot()
        .vault_by_backing(&CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .clone();
    assert_eq!(vault.owner, PrincipalId::Object(who));
    assert_eq!(vault.name, "FPC-D-0401 hold");
    // The hull: renamed, the mark kept, described under its own owner.
    assert_eq!(name(&rig.tw, hull), "Salt Moth");
    assert!(
        tw_aliases(&rig.tw, hull).iter().any(|a| a == "FPC-D-0401"),
        "{:?}",
        tw_aliases(&rig.tw, hull)
    );
    assert_eq!(
        attr_text(&rig.tw, hull, "desc").as_deref(),
        Some(
            "A Dray-class bulk hauler, FPC-D-0401, cradled with its umbilicals slack: blunt, \
             grey and webbed with reaction-mass hoses."
        )
    );
    let hull_owner = rig.tw.world().get(hull).expect("the hull").owner;
    assert_eq!(
        rig.tw.world().attr(hull, "desc").expect("a desc").owner,
        hull_owner,
        "the description is the hull owner's, never the pilot's"
    );
    assert_ne!(hull_owner, who);
    // The enrolment document.
    let account_id = account("PilotAcct", 1);
    let e = match store(&rig.tw).enrolment_state(account_id) {
        EnrolmentState::Present(e) => e.clone(),
        other => panic!("the enrolment: {other:?}"),
    };
    assert_eq!(e.pilot, who);
    assert_eq!(e.hull, hull);
    assert_eq!(e.mark, "FPC-D-0401");
    assert_eq!(e.laid_up_name, "Slack Water");
    assert_eq!(e.name, "Salt Moth");
    assert_eq!(e.lease, lease.0);
    assert_eq!(e.vault, vault.id.0);
    assert_eq!(e.credit_account, acct.0);
    assert_eq!(e.stake_minor, 2_500_000);
    assert_eq!(e.share_permille, 250);
    assert_eq!(e.buyout_minor, 9_000_000);
    assert_eq!(e.status, EnrolmentStatus::Active);
    assert_eq!((e.sales, e.margin_minor, e.shares_minor), (0, 0, 0));
    assert_eq!(e.programme, cid("coop.first_captain"));
    assert_eq!(e.entry, cid("ceres_port"));
    // `+coop` now.
    let terms = run(&mut rig.tw, who, "+coop");
    let registrar = format!("The registrar says, \"{REGISTRAR_LINE}\"");
    assert!(
        has_block(
            &terms,
            &[
                registrar.as_str(),
                "Terms: a 25000.00cr stake; 250‰ of trading margin over the Co-op's reference \
                 prices; buyout 90000.00cr, fixed at signing (a new Dray from the Yard is \
                 120000.00cr); one enrolment per pilot, ever.",
                "Drays free: 7 of 8.",
                "Your charter: the Salt Moth (FPC-D-0401); buyout 90000.00cr. +coop/statement \
                 reads your lay.",
            ]
        ),
        "{terms}"
    );
    // `+coop/statement`, in force: the frozen terms, zero counters and the
    // staked cash.
    let statement = run(&mut rig.tw, who, "+coop/statement");
    assert!(
        has_block(
            &statement,
            &[
                "Statement: the Salt Moth (FPC-D-0401), First Captain programme.",
                "  Terms: stake 25000.00cr; 250‰ of trading margin; buyout 90000.00cr.",
                "  Sales: 0; margin 0.00cr; Co-op share paid 0.00cr.",
                "  Cash: 25000.00cr available; the buyout is 90000.00cr.",
                "  Charter: in force.",
            ]
        ),
        "{statement}"
    );
    // The readiness hulls line learns the leased arm and stays ready.
    let report = run(&mut rig.tw, GOD, "@tacsim/port status");
    assert!(
        report.contains(
            "ceres_port hulls: 8 of 8 made and titled to Co-op First Captain Trust; 7 laid up and \
             free, 1 leased (FPC-D-0401 to Pilot). [ready]"
        ),
        "{report}"
    );
    assert!(enrol_gate(&readiness(rig.tw.world())).is_ok());
}

/// The aliases of `obj`.
fn tw_aliases(tw: &TestWorld, obj: ObjId) -> Vec<String> {
    tw.world()
        .get(obj)
        .map(|o| o.aliases.iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 2. One enrolment per gateway account, ever
// ---------------------------------------------------------------------------

#[test]
fn ic6_a_second_enrolment_from_the_same_account_is_refused_for_ever() {
    let (mut rig, who) = enrolled();
    let before = counts(&rig.tw);
    let again = enrol(&mut rig, who, "Second Wind");
    assert!(again.contains(ENROLLED_ONCE), "{again}");
    assert_eq!(counts(&rig.tw), before);
    // A second character on the same gateway account.
    let alt = pilot_on(&mut rig, "Alt", account("PilotAcct", 1));
    let alt_out = enrol(&mut rig, alt, "Second Wind");
    assert!(alt_out.contains(ENROLLED_ONCE), "{alt_out}");
    assert_eq!(counts(&rig.tw), before);
    assert_eq!(own_account(&rig.tw, alt), None, "nothing was opened");
    // `+coop` reads the books by gateway account: the alt is told the account
    // is on them, never invited to an enrolment the desk refuses.
    let alt_terms = run(&mut rig.tw, alt, "+coop");
    assert!(alt_terms.contains(ENROLLED_ONCE), "{alt_terms}");
    assert!(!alt_terms.contains("Enrol at"), "{alt_terms}");
    // A pilot on another account is chartered the next hull.
    let other = pilot(&mut rig, "Other", 2);
    let out = enrol(&mut rig, other, "Second Wind");
    assert!(
        out.contains(
            "The Co-op charters you FPC-D-0402, laid up as the Cold Comfort, now the \
                      Second Wind"
        ),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// 3. The refusals before any write
// ---------------------------------------------------------------------------

#[test]
fn ic6_enrol_refuses_away_from_the_desk_and_without_its_prerequisites() {
    // Before the first sweep the capital is pending: the desk is not open.
    let mut rig = boot_unsettled(Roster::bare());
    let early = pilot(&mut rig, "Early", 9);
    let before = counts(&rig.tw);
    let out = enrol(&mut rig, early, "Salt Moth");
    assert!(out.contains(NOT_OPEN), "{out}");
    assert_eq!(counts(&rig.tw), before);

    let mut rig = boot(Roster::bare());
    let who = pilot(&mut rig, "Pilot", 1);
    let before = counts(&rig.tw);
    // Away from the desk.
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(
        out.contains("Enrolment is signed at the Registry & Co-op Desk in Ceres Freeport."),
        "{out}"
    );
    // `+coop` anywhere invites the unenrolled to the desk.
    let terms = run(&mut rig.tw, who, "+coop");
    assert!(
        terms.contains(
            "Enrol at the Registry & Co-op Desk in Ceres Freeport: +coop/enrol <a name for your \
             ship>."
        ),
        "{terms}"
    );
    let desk = c05(&rig.tw);
    teleport(&mut rig.tw, who, desk);
    // No name.
    let out = run(&mut rig.tw, who, "+coop/enrol");
    assert!(out.contains(NAME_USAGE), "{out}");
    // A bad name.
    for bad in [
        "2nd Wind",
        "Salt/Moth",
        "A name far too long for any registry",
    ] {
        let out = enrol(&mut rig, who, bad);
        assert!(out.contains(BAD_NAME), "{bad}: {out}");
    }
    // A taken name: a laid-up hull's, any case; and a mark.
    for taken in ["Cold Comfort", "cold comfort", "FPC-D-0405"] {
        let out = enrol(&mut rig, who, taken);
        assert!(out.contains(NAME_TAKEN), "{taken}: {out}");
    }
    // An unmapped character.
    let stranger = rig.tw.spawn_player("Stranger");
    teleport(&mut rig.tw, stranger, desk);
    let out = enrol(&mut rig, stranger, "Salt Moth");
    assert!(out.contains(NO_ACCOUNT_MATCH), "{out}");
    assert_eq!(counts(&rig.tw), before, "a refusal wrote something");
    assert_eq!(own_account(&rig.tw, who), None);
}

// ---------------------------------------------------------------------------
// 4. The rate gate on a production layer
// ---------------------------------------------------------------------------

/// The live world rate.
fn world_rate(tw: &TestWorld) -> u32 {
    tw.world()
        .world_timeline()
        .epochs()
        .last()
        .map(|epoch| epoch.rate)
        .expect("a world-time generation")
}

/// Open one rate generation at `rate` when the live one differs (`ports.rs`'s).
fn set_rate(tw: &mut TestWorld, rate: u32) {
    if world_rate(tw) != rate {
        let out = run(tw, GOD, &format!("@tacsim/time rate={rate}"));
        assert_eq!(world_rate(tw), rate, "{out}");
    }
}

#[test]
fn ic6_the_desk_closes_at_a_play_world_rate_other_than_24() {
    let mut rig = boot_unsettled(Roster::bare_real());
    sweep(&mut rig.tw, 120);
    // Doc 18 cut IC8: the ports stocked BEFORE the rate moves, so the rate is the one
    // binding that closes the desk (fixer T1: an unstocked port would close it too).
    // Doc 18 cut IC12: the stocking helper waits for the whole gate (every line but
    // the sky), which a real world reads only at the play rate — so the world is put
    // at 24 first, as dev1's deploy step puts it, and then moved off it.
    set_rate(&mut rig.tw, 24);
    ensure_stocked(&mut rig);
    set_rate(&mut rig.tw, 4);
    let report = run(&mut rig.tw, GOD, "@tacsim/port status");
    let closed: Vec<&str> = report
        .lines()
        .filter(|line| line.ends_with(']') && !line.ends_with("[ready]"))
        // doc 20 SM.1: the geology line is reported and never gated
        .filter(|line| !line.starts_with("geology: "))
        .collect();
    assert!(
        !closed.is_empty() && closed.iter().all(|line| line.contains("world rate")),
        "only the rate line is not ready:\n{report}"
    );
    let who = pilot(&mut rig, "Pilot", 1);
    let before = counts(&rig.tw);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains(NOT_OPEN), "{out}");
    assert_eq!(counts(&rig.tw), before);
    set_rate(&mut rig.tw, 24);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains("now the Salt Moth"), "{out}");
}

/// **The desk stays closed while the ports are unstocked** (doc 18 cut IC8, lead
/// ruling L8; §4.7 "IC8 adds pools seeded and listings equal to pools"): with the
/// capital settled and the rate at play, an enrol before the port tick has seeded and
/// listed the stock is refused `NOT_OPEN` and writes nothing, and the report names
/// the pending stock lines; once the clerks have stocked both ports it succeeds.
#[test]
fn ic8_the_desk_stays_closed_until_the_ports_are_stocked() {
    let mut rig = boot_unsettled(Roster::bare());
    sweep(&mut rig.tw, 120);
    assert!(
        enrol_gate(&readiness(rig.tw.world())).is_ok(),
        "the world-only bindings are open:\n{}",
        readiness(rig.tw.world()).render().join("\n")
    );
    let report = run(&mut rig.tw, GOD, "@tacsim/port status");
    let closed: Vec<&str> = report
        .lines()
        .filter(|line| line.ends_with(']') && !line.ends_with("[ready]"))
        // doc 20 SM.1: the geology line is reported and never gated
        .filter(|line| !line.starts_with("geology: "))
        .collect();
    assert!(
        !closed.is_empty()
            && closed
                .iter()
                .all(|line| line.contains(" vendor ") || line.contains(" ask ")),
        "only stock lines hold the desk:\n{report}"
    );
    assert!(
        closed.iter().any(|line| line.contains(" vendor ")),
        "a Pools line is pending:\n{report}"
    );
    let who = pilot(&mut rig, "Pilot", 1);
    let before = counts(&rig.tw);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains(NOT_OPEN), "{out}");
    assert_eq!(counts(&rig.tw), before, "a refused enrol wrote something");
    ensure_stocked(&mut rig);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains("now the Salt Moth"), "{out}");
}

// ---------------------------------------------------------------------------
// 5–6. The pool and the capital
// ---------------------------------------------------------------------------

#[test]
fn ic6_the_ninth_pilot_finds_no_dray_free() {
    let mut rig = boot(Roster::bare());
    let names = [
        "Salt Moth",
        "Grey Heron",
        "Iron Kettle",
        "Blue Tern",
        "Late Lark",
        "Red Vane",
        "Low Tide",
        "Night Owl",
    ];
    for (n, ship) in names.iter().enumerate() {
        let seed = i64::try_from(n).expect("small") + 1;
        let who = pilot(&mut rig, &format!("Pilot{seed}"), seed);
        let out = enrol(&mut rig, who, ship);
        let mark = format!("FPC-D-04{seed:02}");
        assert!(
            out.contains(&format!("The Co-op charters you {mark},")),
            "{ship}: {out}"
        );
    }
    let trust_acct = trust_account(&rig.tw);
    assert_eq!(
        balance(&rig.tw, trust_acct),
        5_000_000,
        "the Trust is at its floor"
    );
    let before = counts(&rig.tw);
    let ninth = pilot(&mut rig, "Ninth", 99);
    let out = enrol(&mut rig, ninth, "Last Chance");
    assert!(out.contains(NO_HULL), "{out}");
    assert_eq!(counts(&rig.tw), before);
    assert_eq!(balance(&rig.tw, trust_acct), 5_000_000);
    let terms = run(&mut rig.tw, ninth, "+coop");
    assert!(terms.contains("Drays free: 0 of 8."), "{terms}");
}

#[test]
fn ic6_capital_at_the_floor_refuses_the_stake() {
    let mut rig = boot(Roster::bare());
    let trust_acct = trust_account(&rig.tw);
    let treasury =
        mush_economy::AccountId(ceres_map(&rig.tw).treasury.expect("the treasury account"));
    // Leave 74,999.99cr: one stake would breach the 50,000.00cr floor.
    let drain = balance(&rig.tw, trust_acct) - 7_499_999;
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {}={} {drain}", trust_acct.0, treasury.0),
    );
    assert_eq!(balance(&rig.tw, trust_acct), 7_499_999, "{out}");
    let who = pilot(&mut rig, "Pilot", 1);
    let before = counts(&rig.tw);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains(NO_CAPITAL), "{out}");
    assert_eq!(counts(&rig.tw), before);
    assert_eq!(own_account(&rig.tw, who), None);
}

// ---------------------------------------------------------------------------
// 7–8. Boarding and leaving
// ---------------------------------------------------------------------------

/// The domain a port map records.
fn domain_of(tw: &TestWorld, entry: &str) -> u64 {
    port_map(tw, entry).domain.expect("a domain")
}

/// GOD re-places `hull` onto domain `domain`.
fn replace_hull(tw: &mut TestWorld, hull: ObjId, domain: u64) {
    let out = run(
        tw,
        GOD,
        &format!("@spatial/place/domain #{}={domain}", hull.0),
    );
    assert!(out.contains("Placed"), "{out}");
}

fn placement(tw: &TestWorld, obj: ObjId) -> mush_sim::data::SpatialParent {
    tw.world()
        .module_state::<mush_sim::data::SpatialState>()
        .expect("sim state")
        .parent_of(obj)
}

#[test]
fn ic6_board_and_leave_only_by_the_tenant_and_only_docked() {
    let (mut rig, who) = enrolled();
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let hull = ObjId(row.hull);
    let rooms = hull_rooms(&row);
    let gallery = c01(&rig.tw);
    // An onlooker in the gallery, and a visitor GOD placed in the Airlock.
    let other = pilot(&mut rig, "Other", 2);
    teleport(&mut rig.tw, other, gallery);
    let visitor = rig.tw.spawn_player("Visitor");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@behavior/add #{}=sim.participant", visitor.0),
    );
    assert!(!out.contains("denied"), "{out}");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@spatial/place/interior #{}=#{},berths",
            visitor.0, rooms["airlock"].0
        ),
    );
    assert!(out.contains("Placed"), "{out}");
    assert_eq!(location(&rig.tw, visitor), Some(rooms["airlock"]));
    // The tenant boards from C01; each side of the lock sees its own line.
    teleport(&mut rig.tw, who, gallery);
    let events = rig.tw.run(who, "+ship/board");
    let out = said(&rig.tw, &events, who);
    assert!(
        out.contains("You cycle the lock into the Salt Moth."),
        "{out}"
    );
    assert!(
        !out.contains("Pilot cycles"),
        "the actor is excluded:\n{out}"
    );
    let seen = said(&rig.tw, &events, other);
    assert!(
        seen.contains("Pilot cycles through the lock into the Salt Moth."),
        "the gallery:\n{seen}"
    );
    assert!(!seen.contains("comes aboard"), "the gallery:\n{seen}");
    let seen = said(&rig.tw, &events, visitor);
    assert!(
        seen.contains("Pilot comes aboard through the outer door."),
        "the Airlock:\n{seen}"
    );
    assert!(!seen.contains("cycles through"), "the Airlock:\n{seen}");
    assert_eq!(location(&rig.tw, who), Some(rooms["airlock"]));
    assert!(matches!(
        placement(&rig.tw, who),
        mush_sim::data::SpatialParent::Interior { space, slot: Some(ref slot) }
            if space == rooms["airlock"] && slot.as_str() == "berths"
    ));
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(out.contains("You are already aboard."), "{out}");
    // A non-tenant in C01 is refused.
    let out = run(&mut rig.tw, other, "+ship/board");
    assert!(out.contains("You hold no charter."), "{out}");
    assert_eq!(location(&rig.tw, other), Some(gallery));
    // Leave from the Central Passage is refused, naming the room.
    let _ = run(&mut rig.tw, who, "in");
    assert_eq!(location(&rig.tw, who), Some(rooms["passage"]));
    let out = run(&mut rig.tw, who, "+ship/leave");
    assert!(out.contains("You can only leave by the Airlock."), "{out}");
    assert_eq!(location(&rig.tw, who), Some(rooms["passage"]));
    let _ = run(&mut rig.tw, who, "airlock");
    assert_eq!(location(&rig.tw, who), Some(rooms["airlock"]));
    // Leave from the Airlock lands in C01, unplaced; each side of the lock
    // sees its own line.
    let events = rig.tw.run(who, "+ship/leave");
    let out = said(&rig.tw, &events, who);
    assert!(
        out.contains("You cycle the lock out into the Visiting Berth Gallery."),
        "{out}"
    );
    assert!(
        !out.contains("Pilot comes ashore"),
        "the actor is excluded:\n{out}"
    );
    let seen = said(&rig.tw, &events, visitor);
    assert!(
        seen.contains("Pilot cycles out through the outer door."),
        "the Airlock:\n{seen}"
    );
    assert!(!seen.contains("comes ashore"), "the Airlock:\n{seen}");
    let seen = said(&rig.tw, &events, other);
    assert!(
        seen.contains("Pilot comes ashore from the Salt Moth."),
        "the gallery:\n{seen}"
    );
    assert!(!seen.contains("cycles out"), "the gallery:\n{seen}");
    assert_eq!(location(&rig.tw, who), Some(gallery));
    assert!(matches!(
        placement(&rig.tw, who),
        mush_sim::data::SpatialParent::Unplaced
    ));
    let out = run(&mut rig.tw, who, "+ship/leave");
    assert!(out.contains("You are not aboard a Dray."), "{out}");
    // The tenant's board from C03 is refused.
    let arrivals = c03(&rig.tw);
    teleport(&mut rig.tw, who, arrivals);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(
        out.contains("You board from the berth gallery where your Dray is cradled."),
        "{out}"
    );
    // The hull at Stickney: C01 refuses, P01 boards.
    let stickney = domain_of(&rig.tw, "stickney_port");
    replace_hull(&mut rig.tw, hull, stickney);
    teleport(&mut rig.tw, who, gallery);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(
        out.contains("The Salt Moth is not cradled at this gallery."),
        "{out}"
    );
    let stickney_gallery = p01(&rig.tw);
    teleport(&mut rig.tw, who, stickney_gallery);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(
        out.contains("You cycle the lock into the Salt Moth."),
        "{out}"
    );
    assert_eq!(location(&rig.tw, who), Some(rooms["airlock"]));
    // The hull on a non-port domain: leave refused, the tenant still aboard.
    let field = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Room, "Open Field", GOD, None)
        .expect("a room");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@behavior/add #{}=sim.domain", field.0),
    );
    assert!(!out.contains("denied"), "{out}");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    let domain = rig
        .tw
        .world()
        .module_state::<mush_sim::data::SpatialState>()
        .expect("sim")
        .domain_of_host(field)
        .map(|d| d.id.0)
        .unwrap_or_else(|| panic!("a domain on the field: {out}"));
    replace_hull(&mut rig.tw, hull, domain);
    let out = run(&mut rig.tw, who, "+ship/leave");
    assert!(
        out.contains("The Salt Moth is not at a berth; the lock stays sealed."),
        "{out}"
    );
    assert_eq!(location(&rig.tw, who), Some(rooms["airlock"]));
}

#[test]
fn ic6_the_stamp_follows_leave_and_the_ceres_door() {
    let (mut rig, who) = enrolled();
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let hull = ObjId(row.hull);
    let gallery = c01(&rig.tw);
    let stickney_gallery = p01(&rig.tw);
    let marks = |tw: &TestWorld| {
        (
            tw.world().has_marker(who, "tacsim.last_port.ceres"),
            tw.world().has_marker(who, "tacsim.last_port.stickney"),
        )
    };
    // Boarding is a `Moved` out of a port room into a room in no port, so
    // IC1's `IcReturnStamp` stamps the boarder: Ceres.
    teleport(&mut rig.tw, who, gallery);
    let _ = run(&mut rig.tw, who, "+ship/board");
    assert_eq!(
        marks(&rig.tw),
        (true, false),
        "IC1's observer stamped the board"
    );
    // The hull re-placed onto Stickney with the tenant aboard: no `Moved` out
    // of a port room happens on the leave (its `from` is the airlock), so only
    // `+ship/leave`'s own stamp can set Stickney and clear Ceres.
    let stickney = domain_of(&rig.tw, "stickney_port");
    replace_hull(&mut rig.tw, hull, stickney);
    let _ = run(&mut rig.tw, who, "+ship/leave");
    assert_eq!(location(&rig.tw, who), Some(stickney_gallery));
    assert_eq!(
        marks(&rig.tw),
        (false, true),
        "+ship/leave stamped Stickney"
    );
    // The reverse. First carry a Ceres stamp into P01 (the C03 door pass,
    // IC1's observer; a teleport between port rooms, or out of Limbo, stamps
    // nothing), so the board at P01 has something to flip.
    let arrivals = c03(&rig.tw);
    teleport(&mut rig.tw, who, arrivals);
    assert_eq!(
        marks(&rig.tw),
        (false, true),
        "a port-to-port move stamps nothing"
    );
    let _ = run(&mut rig.tw, who, "x");
    assert_eq!(location(&rig.tw, who), Some(LIMBO));
    assert_eq!(marks(&rig.tw), (true, false), "the door pass stamped Ceres");
    teleport(&mut rig.tw, who, stickney_gallery);
    assert_eq!(
        marks(&rig.tw),
        (true, false),
        "a move out of Limbo stamps nothing"
    );
    let _ = run(&mut rig.tw, who, "+ship/board");
    assert_eq!(location(&rig.tw, who), Some(hull_rooms(&row)["airlock"]));
    assert_eq!(
        marks(&rig.tw),
        (false, true),
        "IC1's observer stamped the board out of P01"
    );
    let ceres = domain_of(&rig.tw, "ceres_port");
    replace_hull(&mut rig.tw, hull, ceres);
    let _ = run(&mut rig.tw, who, "+ship/leave");
    assert_eq!(location(&rig.tw, who), Some(gallery));
    assert_eq!(marks(&rig.tw), (true, false), "+ship/leave stamped Ceres");
    // The door pass: out of C03 to Limbo stamps Ceres (IC1's observer).
    let _ = run(&mut rig.tw, who, "+ship/board");
    replace_hull(&mut rig.tw, hull, stickney);
    let _ = run(&mut rig.tw, who, "+ship/leave");
    assert_eq!(marks(&rig.tw), (false, true));
    let arrivals = c03(&rig.tw);
    teleport(&mut rig.tw, who, arrivals);
    let _ = run(&mut rig.tw, who, "x");
    assert_eq!(location(&rig.tw, who), Some(LIMBO));
    assert_eq!(marks(&rig.tw), (true, false), "the door pass stamped Ceres");
}

// ---------------------------------------------------------------------------
// 9–10. The lease over time, and a staff cancel
// ---------------------------------------------------------------------------

#[test]
fn ic6_eight_simulated_days_raise_no_stale_contract_page() {
    let (mut rig, who) = enrolled();
    let lease = lease_of(&rig.tw, who);
    // Doc 18 cut IC13: eight idle days are past the programme's seven, so a docked,
    // empty Dray at Ceres would be taken back by the port tick's dormancy recall in
    // the same drain as the stale scan — and a Cancelled lease is never scanned, which
    // would make this guard vacuous. The hull is re-placed at Stickney, where the
    // recall is blocked (`RecallBlock::NotAtPort`), so the lease is still live and
    // eight days quiet when the economy's stale scan reads it.
    let hull = ObjId(ceres_map(&rig.tw).hulls[0].hull);
    let stickney = domain_of(&rig.tw, "stickney_port");
    replace_hull(&mut rig.tw, hull, stickney);
    sweep(&mut rig.tw, 8 * 86_400 + 1);
    let paged = economy(&rig.tw)
        .escalations()
        .filter(|e| e.subject == EscalationSubject::Contract(lease))
        .count();
    assert_eq!(paged, 0, "the live lease was paged as stale");
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .contract(lease)
            .expect("the lease")
            .state,
        ContractState::Active
    );
    let e = store(&rig.tw).enrolment_of(who).expect("the enrolment");
    assert_eq!(e.status, EnrolmentStatus::Active);
    assert_eq!(e.ended, None);
    // The recall arm did judge her dormant (the eight days were real to it) and was
    // blocked where she lies: its page names the hull.
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0],
        "the dormant hull away from Ceres is paged by the recall arm"
    );
}

#[test]
fn ic6_staff_cancel_revokes_the_charter_and_the_gate_reads_the_lease() {
    let (mut rig, who) = enrolled();
    let lease = lease_of(&rig.tw, who);
    let events = rig.tw.run(GOD, &format!("@contract/cancel #{}", lease.0));
    let told = said(&rig.tw, &events, who);
    let notice = "The Co-op has ended your charter of the Salt Moth (FPC-D-0401).";
    assert!(
        told.contains(notice)
            || pending_notices(rig.tw.world(), who)
                .iter()
                .any(|n| n == notice),
        "the pilot was not told: {told}"
    );
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .contract(lease)
            .expect("the lease")
            .state,
        ContractState::Cancelled
    );
    let e = store(&rig.tw)
        .enrolment_of(who)
        .expect("the enrolment")
        .clone();
    assert_eq!(e.status, EnrolmentStatus::Revoked);
    assert_eq!(tacsim::port::live_charter(rig.tw.world(), who), None);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(out.contains("You hold no charter."), "{out}");
    let ended = "Your charter of the Salt Moth (FPC-D-0401) has ended.";
    let out = run(&mut rig.tw, who, "+ship");
    assert!(out.contains(ended), "{out}");
    let out = run(&mut rig.tw, who, "+coop");
    assert!(out.contains(ended), "{out}");
    // The revoked hull's hold still stands (L9): seven free.
    assert!(out.contains("Drays free: 7 of 8."), "{out}");
    let statement = run(&mut rig.tw, who, "+coop/statement");
    assert!(statement.contains("Charter: ended."), "{statement}");
    // With the rent cancelled, staff empty and close the pilot's account: the
    // statement's cash line says so in a sentence of its own.
    let acct = own_account(&rig.tw, who).expect("the pilot's account");
    let trust_acct = trust_account(&rig.tw);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {}={} 2500000", acct.0, trust_acct.0),
    );
    assert_eq!(balance(&rig.tw, acct), 0, "{out}");
    let out = run(&mut rig.tw, GOD, &format!("@account/close {}", acct.0));
    assert_eq!(own_account(&rig.tw, who), None, "{out}");
    let statement = run(&mut rig.tw, who, "+coop/statement");
    assert!(
        statement.contains("  Cash: no account is open; the buyout is 90000.00cr."),
        "{statement}"
    );
    let desk = c05(&rig.tw);
    teleport(&mut rig.tw, who, desk);
    let out = enrol(&mut rig, who, "Second Wind");
    assert!(out.contains(ENROLLED_ONCE), "{out}");
}

/// The readiness report's Ceres hulls line.
fn hulls_line(rig: &mut Rig) -> String {
    let report = run(&mut rig.tw, GOD, "@tacsim/port status");
    report
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("ceres_port hulls: "))
        .unwrap_or_else(|| {
            panic!(
                "a hulls line:
{report}"
            )
        })
        .to_owned()
}

/// Doc 18 cut IC13 item 3 (lead ruling L2): the operate gate is the live charter, else
/// the title; `helm_holder` names the charter's tenant or the title holder, and nobody
/// for a laid-up Trust hull. The title arm is reached here through a staff
/// `@asset/transfer` of a laid-up unleased hull; the readiness line counts it as a
/// graduate and stays Ready.
#[test]
fn ic13_the_helm_is_the_charter_or_the_title() {
    use tacsim::port::{Helm, Standing, helm_holder, helm_of};
    let (mut rig, who) = enrolled();
    let charter = tacsim::port::live_charter(rig.tw.world(), who).expect("a charter");
    assert_eq!(
        helm_of(rig.tw.world(), who),
        Some(Helm {
            hull: charter.hull,
            standing: Standing::Charter(charter.contract),
        })
    );
    assert_eq!(helm_holder(rig.tw.world(), charter.hull), Some(who));
    let map = ceres_map(&rig.tw);
    let mut laid_up = map
        .hulls
        .iter()
        .map(|row| ObjId(row.hull))
        .filter(|hull| *hull != charter.hull);
    let titled = laid_up.next().expect("a second hull");
    let idle = laid_up.next().expect("a third hull");
    // A Trust-titled unleased hull has no holder.
    assert_eq!(helm_holder(rig.tw.world(), titled), None);
    assert_eq!(helm_holder(rig.tw.world(), idle), None);
    let other = pilot(&mut rig, "Other", 2);
    assert_eq!(helm_of(rig.tw.world(), other), None);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", titled.0, other.0),
    );
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .asset(titled)
            .map(|a| a.title_owner.clone()),
        Some(PrincipalId::Object(other)),
        "{out}"
    );
    assert_eq!(
        helm_of(rig.tw.world(), other),
        Some(Helm {
            hull: titled,
            standing: Standing::Title,
        })
    );
    assert_eq!(helm_holder(rig.tw.world(), titled), Some(other));
    assert_eq!(helm_holder(rig.tw.world(), idle), None);
    // The chartered pilot's helm is unchanged.
    assert_eq!(
        helm_of(rig.tw.world(), who).map(|h| h.standing),
        Some(Standing::Charter(charter.contract))
    );
    // The hulls line counts the title holder's hull as graduated, and stays Ready.
    let mark = map
        .hulls
        .iter()
        .find(|row| row.hull == titled.0)
        .map(|row| row.mark.clone())
        .expect("the mark");
    assert_eq!(
        hulls_line(&mut rig),
        format!(
            "ceres_port hulls: 8 of 8 made, 7 titled to Co-op First Captain Trust; 6 laid up and free, 1 leased (FPC-D-0401 to Pilot), 1 graduated ({mark} to Other). [ready]"
        )
    );
    assert!(enrol_gate(&readiness(rig.tw.world())).is_ok());
}

/// Doc 18 cut IC13 items 7–8 (lead ruling L4; closes IC6's residual 3): a staff-cancelled
/// charter leaves its hull "recalled, not yet returned" on the hulls line (Ready, never
/// free), until the port tick's Return arm, finding her docked at Ceres with an empty
/// hold, re-homes the hold vault to the Trust, restores the laid-up name and puts her
/// back in the pool.
#[test]
fn ic13_a_staff_cancelled_hull_reads_recalled_until_the_return_arm_takes_it_back() {
    let (mut rig, who) = enrolled();
    let lease = lease_of(&rig.tw, who);
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let _ = rig.tw.run(GOD, &format!("@contract/cancel #{}", lease.0));
    let e = store(&rig.tw)
        .enrolment_of(who)
        .expect("the enrolment")
        .clone();
    assert_eq!(e.status, EnrolmentStatus::Revoked);
    assert_eq!(e.ended, Some(tacsim::port::EndCause::LeaseEnded));
    assert!(e.ended_at.is_some());
    assert_eq!(
        hulls_line(&mut rig),
        "ceres_port hulls: 8 of 8 made and titled to Co-op First Captain Trust; 7 laid up and free, 0 leased, 1 recalled, not yet returned (FPC-D-0401). [ready]"
    );
    assert!(enrol_gate(&readiness(rig.tw.world())).is_ok());
    // The Return arm on the next port-tick fire.
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let told = said(&rig.tw, &events, who);
    let v = economy(&rig.tw)
        .snapshot()
        .vault(vault)
        .expect("the hold vault")
        .clone();
    assert_eq!(v.owner, trust(&rig.tw));
    assert_eq!(v.status, mush_economy::VaultStatus::Active);
    assert_eq!(name(&rig.tw, hull), "Slack Water");
    let returned = "The Co-op has taken the Salt Moth (FPC-D-0401) back into its pool.";
    assert!(
        told.contains(returned)
            || pending_notices(rig.tw.world(), who)
                .iter()
                .any(|n| n == returned),
        "the pilot was not told: {told}"
    );
    assert_eq!(
        hulls_line(&mut rig),
        "ceres_port hulls: 8 of 8 made, titled to Co-op First Captain Trust and laid up. [ready]"
    );
    let out = run(&mut rig.tw, who, "+coop");
    assert!(out.contains("Drays free: 8 of 8."), "{out}");
}

// ---------------------------------------------------------------------------
// 11–12. The look hook and the gauges
// ---------------------------------------------------------------------------

#[test]
fn ic6_look_names_the_hull_and_the_port() {
    let (mut rig, who) = enrolled();
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let rooms = hull_rooms(&row);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let _ = run(&mut rig.tw, who, "+ship/board");
    let status = "Aboard the Salt Moth (FPC-D-0401), docked at Ceres Freeport.";
    // Walk every walkable room and look.
    for (step, room) in [
        (None, "airlock"),
        (Some("in"), "passage"),
        (Some("bridge"), "bridge"),
        (Some("passage"), "passage"),
        (Some("crew"), "crew"),
        (Some("passage"), "passage"),
        (Some("hold"), "hold"),
        (Some("eng"), "engineering"),
    ] {
        if let Some(step) = step {
            let _ = run(&mut rig.tw, who, step);
        }
        assert_eq!(location(&rig.tw, who), Some(rooms[room]), "{room}");
        let out = run(&mut rig.tw, who, "look");
        assert_eq!(out.matches(status).count(), 1, "{room}:\n{out}");
        // The status line precedes every gauge line.
        let at = out.find(status).expect("the status line");
        for label in [
            "Load cells:",
            "Manifest slate:",
            "Displays:",
            "Locker tally:",
            "Tank gauges:",
        ] {
            if let Some(gauge) = out.find(label) {
                assert!(at < gauge, "{room}: {label} before the status line:\n{out}");
            }
        }
    }
    // A laid-up hull: *Cold Comfort*, GOD placed in its airlock.
    let laid_up = ceres_map(&rig.tw).hulls[1].clone();
    let cold = hull_rooms(&laid_up);
    let other = rig.tw.spawn_player("Visitor");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@behavior/add #{}=sim.participant", other.0),
    );
    assert!(!out.contains("denied"), "{out}");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@spatial/place/interior #{}=#{},berths",
            other.0, cold["airlock"].0
        ),
    );
    assert!(out.contains("Placed"), "{out}");
    let out = run(&mut rig.tw, other, "look");
    assert!(
        out.contains("Aboard the Cold Comfort (FPC-D-0402), docked at Ceres Freeport."),
        "{out}"
    );
    for label in [
        "Load cells:",
        "Manifest slate:",
        "Displays:",
        "Locker tally:",
        "Tank gauges:",
    ] {
        assert!(!out.contains(label), "the Airlock shows a gauge:\n{out}");
    }
    let _ = run(&mut rig.tw, other, "in");
    let _ = run(&mut rig.tw, other, "bridge");
    assert_eq!(location(&rig.tw, other), Some(cold["bridge"]));
    let out = run(&mut rig.tw, other, "look");
    assert!(
        has_block(
            &out,
            &[
                "Aboard the Cold Comfort (FPC-D-0402), docked at Ceres Freeport.",
                "Displays:",
                "  HOLD empty; 0/100 t · 0.0/120 m³",
                "  TANKAGE 0/80 t",
                // Doc 18 cut IC14 row 54: the burn gauge under the tank line.
                "  Burn for the filed plan: none filed.",
                "  STORES 0/120 crew-days",
            ]
        ),
        "{out}"
    );
    // A kept world that turns the layer off keeps its hull rooms, and the
    // hook says nothing.
    let bridge = cold["bridge"];
    let roster = rig.roster.clone().disabled();
    let mut rig = reboot_with(rig, roster, |_| {});
    assert!(rig.tw.world().get(bridge).is_some(), "the hull room stands");
    teleport(&mut rig.tw, GOD, bridge);
    let out = run(&mut rig.tw, GOD, "look");
    // The look rendered the room itself ...
    let bridge_name = name(&rig.tw, bridge);
    assert!(out.contains(&bridge_name), "the Bridge rendered:\n{out}");
    let desc = attr_text(&rig.tw, bridge, "desc").expect("the Bridge has a description");
    assert!(
        out.contains(&desc),
        "the Bridge's description rendered:\n{out}"
    );
    // ... and only the hook went quiet.
    assert!(!out.contains("Aboard the"), "{out}");
    assert!(!out.contains("Displays:"), "{out}");
}

#[test]
fn ic6_the_gauges_read_the_hold_pools() {
    let (mut rig, who) = enrolled();
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let rooms = hull_rooms(&row);
    seed_outbound(&mut rig, &row);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let _ = run(&mut rig.tw, who, "+ship/board");
    let status = "Aboard the Salt Moth (FPC-D-0401), docked at Ceres Freeport.";
    // The Airlock: the status line and no gauge.
    let out = run(&mut rig.tw, who, "look");
    assert!(out.contains(status), "{out}");
    for label in [
        "Load cells:",
        "Manifest slate:",
        "Displays:",
        "Locker tally:",
        "Tank gauges:",
    ] {
        assert!(!out.contains(label), "the Airlock shows a gauge:\n{out}");
    }
    let summary = summary_body();
    let cells = load_cells_block();
    let mut passage: Vec<&str> = vec![status, "Manifest slate:"];
    passage.extend(summary.iter().map(String::as_str));
    // Doc 18 cut IC14 row 54: the Bridge's displays carry the burn gauge under the
    // TANKAGE line; the passage's manifest slate does not.
    let burn_none = format!("  {}", tacsim::port::ship::BURN_GAUGE_NONE);
    let mut bridge: Vec<&str> = vec![status, "Displays:"];
    for line in &summary {
        bridge.push(line.as_str());
        if line.starts_with("  TANKAGE ") {
            bridge.push(burn_none.as_str());
        }
    }
    let mut hold: Vec<&str> = vec![status];
    hold.extend(cells.iter().map(String::as_str));
    for (step, room, block) in [
        ("in", "passage", passage),
        ("bridge", "bridge", bridge),
        ("passage", "passage", Vec::new()),
        (
            "crew",
            "crew",
            vec![
                status,
                "Locker tally: voyage provisions, 19 of 120 crew-days.",
            ],
        ),
        ("passage", "passage", Vec::new()),
        ("hold", "hold", hold),
        (
            "eng",
            "engineering",
            vec![
                status,
                "Tank gauges: reaction mass, 53 of 80 t.",
                tacsim::port::ship::BURN_GAUGE_NONE,
            ],
        ),
    ] {
        let _ = run(&mut rig.tw, who, step);
        assert_eq!(location(&rig.tw, who), Some(rooms[room]), "{room}");
        if block.is_empty() {
            continue;
        }
        let out = run(&mut rig.tw, who, "look");
        assert!(has_block(&out, &block), "{room}:\n{out}");
    }
    // `+ship` and `+ship/hold`.
    let out = run(&mut rig.tw, who, "+ship");
    assert!(
        has_block(
            &out,
            &[
                "Salt Moth (Dray): docked Ceres Freeport, Visiting Berth Gallery.",
                "Hold: cargo 100/100 t · 120.0/120 m³ · tank 53/80 t · locker 19/120 crew-days.",
                "Radio: about 1 IC seconds one way to an Exchange endpoint; queue time and returning acknowledgements are additional.",
                "Programme: 0 sales; buyout 90000.00cr.",
            ]
        ),
        "{out}"
    );
    let out = run(&mut rig.tw, who, "+ship/hold");
    let mut want = vec![
        "Hold of the Salt Moth (FPC-D-0401):".to_owned(),
        "  HOLD".to_owned(),
    ];
    want.extend(load_cells("    "));
    want.push("  TANKAGE reaction mass, 53 of 80 t".to_owned());
    want.push("  STORES voyage provisions, 19 of 120 crew-days".to_owned());
    let want: Vec<&str> = want.iter().map(String::as_str).collect();
    assert!(has_block(&out, &want), "{out}");
}

// ---------------------------------------------------------------------------
// 13. A reboot, and an unreadable enrolment
// ---------------------------------------------------------------------------

#[test]
fn ic6_the_charter_survives_a_reboot() {
    let (mut rig, who) = enrolled();
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let rooms = hull_rooms(&row);
    let hull = ObjId(row.hull);
    seed_outbound(&mut rig, &row);
    let roster = rig.roster.clone();
    let mut rig = reboot_with(rig, roster, |_| {});
    let _ = rig.tw.connect(who);
    assert!(matches!(
        store(&rig.tw).enrolment_state(account("PilotAcct", 1)),
        EnrolmentState::Present(e) if e.pilot == who
    ));
    assert_eq!(name(&rig.tw, hull), "Salt Moth");
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(
        out.contains("You cycle the lock into the Salt Moth."),
        "{out}"
    );
    let _ = run(&mut rig.tw, who, "in");
    let _ = run(&mut rig.tw, who, "hold");
    assert_eq!(location(&rig.tw, who), Some(rooms["hold"]));
    let out = run(&mut rig.tw, who, "look");
    assert!(
        has_block(
            &out,
            &load_cells_block()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        ),
        "the goods cache did not survive:\n{out}"
    );
    let _ = run(&mut rig.tw, who, "passage");
    let _ = run(&mut rig.tw, who, "airlock");
    let _ = run(&mut rig.tw, who, "+ship/leave");
    assert_eq!(location(&rig.tw, who), Some(gallery));

    // Corrupt the chartered pilot's enrolment document and reboot.
    let key = enrolment_key(account("PilotAcct", 1));
    let roster = rig.roster.clone();
    let mut rig = reboot_with(rig, roster, |folded| {
        let doc = folded
            .get_mut(tacsim::NS)
            .and_then(|data| data.kv.iter_mut().find(|(k, _)| *k == key))
            .expect("the enrolment document");
        doc.1 = serde_json::json!({ "schema": 1, "nonsense": true });
    });
    let _ = rig.tw.connect(who);
    assert!(matches!(
        store(&rig.tw).enrolment_state(account("PilotAcct", 1)),
        EnrolmentState::Unreadable
    ));
    // The gate is the lease: `+ship` renders its first two lines, then dark.
    let out = run(&mut rig.tw, who, "+ship");
    assert!(
        has_block(
            &out,
            &[
                "Salt Moth (Dray): docked Ceres Freeport, Visiting Berth Gallery.",
                "Hold: cargo 100/100 t · 120.0/120 m³ · tank 53/80 t · locker 19/120 crew-days.",
                "Programme: the Co-op's books cannot be read.",
            ]
        ),
        "{out}"
    );
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(
        out.contains("You cycle the lock into the Salt Moth."),
        "{out}"
    );
    let _ = run(&mut rig.tw, who, "+ship/leave");
    let out = run(&mut rig.tw, who, "+coop");
    assert!(
        out.contains(
            "Your charter: the Salt Moth (FPC-D-0401); the Co-op's books for your account cannot \
             be read right now."
        ),
        "{out}"
    );
    let out = run(&mut rig.tw, who, "+coop/statement");
    assert!(out.contains(BOOKS_UNREADABLE), "{out}");
    let desk = c05(&rig.tw);
    teleport(&mut rig.tw, who, desk);
    let out = enrol(&mut rig, who, "Second Wind");
    assert!(out.contains(BOOKS_UNREADABLE), "{out}");
    // Another account still enrols.
    let other = pilot(&mut rig, "Other", 2);
    let out = enrol(&mut rig, other, "Second Wind");
    assert!(out.contains("now the Second Wind"), "{out}");
}

// ---------------------------------------------------------------------------
// 14. Retirement refused while a hull is leased
// ---------------------------------------------------------------------------

#[test]
fn ic6_a_port_with_a_leased_hull_refuses_retirement() {
    let (mut rig, _who) = enrolled();
    let out = run(&mut rig.tw, GOD, &format!("@atlas/pack retire {LANE_PACK}"));
    assert!(
        out.contains(
            "ceres_port: hull FPC-D-0401 is chartered to a pilot; a port with a leased hull \
             cannot be retired"
        ),
        "{out}"
    );
    assert!(
        store(&rig.tw)
            .applied_in(rig.tw.world(), &lane_id())
            .is_some()
    );
    // The control: with no hull leased, the port's own retire check refuses
    // nothing.
    let mut idle = boot(Roster::bare());
    let out = run(
        &mut idle.tw,
        GOD,
        &format!("@atlas/pack retire {LANE_PACK}"),
    );
    assert!(!out.contains("chartered to a pilot"), "{out}");
    assert!(out.contains(&format!("Retired {LANE_PACK}")), "{out}");
}

// ---------------------------------------------------------------------------
// 15. A frozen economy refuses enrolment before any write
// ---------------------------------------------------------------------------

#[test]
fn ic6_a_frozen_economy_refuses_enrolment_before_any_write() {
    let mut rig = boot(Roster::bare().freezable());
    let who = pilot(&mut rig, "Pilot", 1);
    let _ = run(&mut rig.tw, GOD, "@econfreeze");
    assert!(economy(&rig.tw).is_frozen(), "the latch is set");
    let before = counts(&rig.tw);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains(FAILED), "{out}");
    // No account was opened in RAM ahead of the offer's frozen refusal.
    assert_eq!(counts(&rig.tw), before, "a frozen refusal wrote something");
    assert_eq!(own_account(&rig.tw, who), None);
    assert!(!out.contains("Ceres Clearing opens account"), "{out}");
    // After the thaw the same pilot enrols, and the account is opened then.
    let out = run(&mut rig.tw, GOD, "@econ/unfreeze");
    assert!(!economy(&rig.tw).is_frozen(), "{out}");
    let out = enrol(&mut rig, who, "Salt Moth");
    let acct = own_account(&rig.tw, who).expect("an account was opened");
    assert!(
        out.contains(&format!(
            "Ceres Clearing opens account #{} in your name.",
            acct.0
        )),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// 16. A disputed lease is not in force, whatever the document says
// ---------------------------------------------------------------------------

#[test]
fn ic6_a_disputed_lease_is_not_in_force() {
    let (mut rig, who) = enrolled();
    let lease = lease_of(&rig.tw, who);
    let out = run(&mut rig.tw, who, &format!("@contract/dispute #{}", lease.0));
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .contract(lease)
            .expect("the lease")
            .state,
        ContractState::Disputed,
        "{out}"
    );
    // The observer folds only an ended lease: the document stays Active.
    assert_eq!(
        store(&rig.tw).enrolment_of(who).map(|e| e.status),
        Some(EnrolmentStatus::Active)
    );
    assert_eq!(tacsim::port::live_charter(rig.tw.world(), who), None);
    // `+coop` and `+coop/statement` read the lease, not the document.
    let terms = run(&mut rig.tw, who, "+coop");
    assert!(
        terms.contains(
            &CHARTER_NOT_IN_FORCE
                .replace("{name}", "Salt Moth")
                .replace("{mark}", "FPC-D-0401")
        ),
        "{terms}"
    );
    assert!(!terms.contains("Enrol at"), "{terms}");
    let statement = run(&mut rig.tw, who, "+coop/statement");
    assert!(
        statement.contains("  Charter: not in force."),
        "{statement}"
    );
    assert!(!statement.contains("Charter: in force."), "{statement}");
    // The operate gate agrees.
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(out.contains("You hold no charter."), "{out}");
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC8: the enrolment checks its document before writing
// ---------------------------------------------------------------------------

/// **The enrolment checks its document before writing** (doc 18 cut IC8, lead 2d;
/// lead ruling L18): a staff rename leaves the lowest free hull with a 130-byte name;
/// enrolment still succeeds, with the laid-up name clamped to the document's 120-byte
/// bound on a char boundary, and the document reloads Present after a reboot.
#[test]
fn ic8_enrolment_checks_its_document_before_writing() {
    let mut rig = boot(Roster::bare());
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let hull = ObjId(row.hull);
    // 119 ASCII bytes, then a two-byte `Å` across the 120-byte bound (bytes 119–120),
    // so a clamp that is not on a char boundary would split it.
    let long = format!("{}Ånd Moré Laid Up", "L".repeat(119));
    assert!(long.len() >= 130, "{}", long.len());
    assert!(!long.is_char_boundary(120), "the bound falls inside a char");
    rig.tw
        .world_mut()
        .set_name(hull, &long)
        .expect("staff rename the hull");
    let who = pilot(&mut rig, "Pilot", 1);
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains("now the Salt Moth"), "{out}");
    let laid_up = match store(&rig.tw).enrolment_state(account("PilotAcct", 1)) {
        EnrolmentState::Present(e) => e.laid_up_name.clone(),
        other => panic!("the enrolment is not present: {other:?}"),
    };
    assert_eq!(
        laid_up.len(),
        119,
        "cut at the last char boundary at or below 120 bytes"
    );
    assert!(long.starts_with(&laid_up), "a prefix of the long name");
    let roster = rig.roster.clone();
    let rig = reboot_with(rig, roster, |_| {});
    assert!(matches!(
        store(&rig.tw).enrolment_state(account("PilotAcct", 1)),
        EnrolmentState::Present(e) if e.laid_up_name == laid_up && e.pilot == who
    ));
}

/// The durable offer-key ring a legal-action offer is signed with — the world task
/// installs it at boot; doc 18 cut IC8's rig lands the port tick's submissions, so it
/// installs one too (`port_tick.rs`'s rig).
fn with_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x41; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// **A refused move leaves the person placed** (doc 18 cut IC14 row 19; lead ruling
/// L12): `put_ashore` moves first and unplaces only once the move landed, so a gallery
/// the person cannot be moved into (here a cycle: the gallery room carried in the
/// pilot's own inventory) refuses `+ship/leave` with the person still in the Airlock
/// and still placed aboard — never unplaced and left behind.
#[test]
fn ic14_a_refused_move_leaves_the_person_placed() {
    let (mut rig, who) = enrolled();
    let row = ceres_map(&rig.tw).hulls[0].clone();
    let rooms = hull_rooms(&row);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(out.contains("You cycle the lock into"), "{out}");
    assert_eq!(location(&rig.tw, who), Some(rooms["airlock"]));
    rig.tw
        .world_mut()
        .move_to(gallery, who)
        .expect("the gallery is carried");
    let out = run(&mut rig.tw, who, "+ship/leave");
    assert!(out.contains(tacsim::port::ship::LOCK_STUCK), "{out}");
    assert_eq!(
        location(&rig.tw, who),
        Some(rooms["airlock"]),
        "still aboard"
    );
    assert!(
        matches!(
            placement(&rig.tw, who),
            mush_sim::data::SpatialParent::Interior { space, .. } if space == rooms["airlock"]
        ),
        "still placed"
    );
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 row 35: the IC6 test gaps
// ---------------------------------------------------------------------------

/// **A failed stake post leaves the recorded residue** (doc 18 cut IC14 row 35; IC6
/// L16). Staff put an approval rule on the Trust's programme account (a real staff
/// act), so `+coop/enrol`'s stake post — the last fallible step, after the lease offer
/// and the pilot's acceptance — answers `NeedsApproval`: the desk answers `FAILED`, and
/// **after a reboot** the durable residue is exactly what IC6 records — the pilot's
/// opened, empty `tacsim.credit` account and a Cancelled lease — and no enrolment.
#[test]
fn ic14_a_failed_stake_post_leaves_the_recorded_residue() {
    let mut rig = boot(Roster::bare());
    let trust_acct = trust_account(&rig.tw);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@account/approval {}=100 threshold:1", trust_acct.0),
    );
    assert!(!out.contains("Usage"), "{out}");
    let who = pilot(&mut rig, "Pilot", 1);
    assert_eq!(
        own_account(&rig.tw, who),
        None,
        "no account before the desk"
    );
    let out = enrol(&mut rig, who, "Salt Moth");
    assert!(out.contains(tacsim::port::coop::FAILED), "{out}");
    let rig = reboot_with(rig, Roster::bare(), |_| {});
    let opened = own_account(&rig.tw, who).expect("the opened account survives");
    assert_eq!(balance(&rig.tw, opened), 0, "empty: the stake never posted");
    let leases: Vec<ContractState> = economy(&rig.tw)
        .snapshot()
        .contracts()
        .filter(|c| c.consent.parties.contains(&PrincipalId::Object(who)))
        .map(|c| c.state)
        .collect();
    assert_eq!(
        leases,
        vec![ContractState::Cancelled],
        "one lease, Cancelled"
    );
    assert!(
        store(&rig.tw).enrolment_of(who).is_none(),
        "no enrolment document"
    );
}

/// **A revocation survives a reboot** (doc 18 cut IC14 row 35): the IC6 staff cancel
/// revokes the charter; folded and rebooted, the enrolment reads Revoked from the
/// store and `+ship/board` still refuses.
#[test]
fn ic14_a_revocation_survives_a_reboot() {
    let (mut rig, who) = enrolled();
    let lease = lease_of(&rig.tw, who);
    let _ = rig.tw.run(GOD, &format!("@contract/cancel #{}", lease.0));
    let before = store(&rig.tw)
        .enrolment_of(who)
        .expect("the enrolment")
        .clone();
    assert_eq!(before.status, EnrolmentStatus::Revoked);
    let mut rig = reboot_with(rig, Roster::bare(), |_| {});
    let after = store(&rig.tw)
        .enrolment_of(who)
        .expect("the enrolment after the reboot")
        .clone();
    assert_eq!(after, before, "the revocation is durable");
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, who, gallery);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(out.contains("You hold no charter."), "{out}");
    assert_eq!(location(&rig.tw, who), Some(gallery));
}
