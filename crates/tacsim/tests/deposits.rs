//! **Doc 20 cut SM.6 acceptance — the industry seams, end to end** (doc 20 §3.1, §3.4, §7.3;
//! D-20-5; the SM.6 brief's acceptance A1–A6, A13 and its tests D1–D10 in 3.14).
//!
//! A site's ore becomes conserved industry quantity before anyone can mine it: at the lane's
//! apply part (g) lays one block vault per block place, and from then on the site's clerk opens
//! one deposit per (block, family) with ore — industry's `DepositOpen`, admitted by the
//! composed `SiteOpener` — and moves each worked block's share into the spoil with
//! `tacsim_site_work`, one act a 5 s port fire, as itself. Until the last row lands the site
//! reads "opening, N of M" and its pad refuses departures. Industry's 60 s sweep then holds
//! every surface pool to its cursor's draws.
//!
//! Composed on the composition root (`knowing.rs`'s keyed rig, copied — the per-file
//! precedent): production modules, the test world's layer, the geology key handed in between
//! composition and the boot pass. Nothing the clerk opens is placed by hand; the site clerks'
//! acts are landed the way the world task lands them (`support::sites::open_sites`). The world
//! runs at rate 1.

mod support;

#[path = "support/geology_keyed.rs"]
mod geology_keyed;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, RequestUid, ScopedIdempotencyKey, World,
};
use mush_economy::{EconomyConfig, VaultId};
use mush_industry::{IndustryFamilies, IndustryHandles, OreDeposit, OreDepositId, Quarantine};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_testkit::{TestWorld, fold_module_ops_into};
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::voyage::SITE_CLOSED;
use tacsim::port::{HullRow, LineState, PortHandles, PortMap, PortMapState, room_key};
use tacsim::surface::cursor::PocketCursor;
use tacsim::surface::geology::{
    BodyGeology, GeologyTable, GeologyTableWire, OreFamily, field_ground,
};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::opening::{OpeningRow, opening_plan};
use tacsim::surface::{GeologyBoot, Word};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{KEY_1, mac, secret};
use support::sites::{SITE_PORTS, open_site_keeping, open_sites_keeping, site_fire, site_line};

/// Ahuna Flank's port entry (an open site, no worked share).
const AHUNA: &str = "ahuna_port";
/// Freeport Commons's port entry (a commons, worked at 400 ‰ in its northern rows).
const COMMONS: &str = "commons_port";

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (knowing.rs's, keyed or not)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Roster;

impl Roster {
    fn modules(self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let llm = Arc::new(
            mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
                .expect("a disabled LLM service composes"),
        );
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let cfg = TacsimConfig {
            enabled: true,
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
        mods.push(comp::tacsim_test_world_module_wired(cfg, true, inventory));
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
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

/// A booted world, its oracle, the cumulative store and the kept typed state.
struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed: BTreeMap<(SmolStr, SmolStr), serde_json::Value>,
    keyed: bool,
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

/// Compose over `world`, hand the key in (or not), then run the boot pass over `store`.
fn boot_over(
    world: World,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    keyed: bool,
) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, Roster.modules())
        .expect("the roster composes");
    if keyed {
        assert!(tacsim::surface::install_geology(
            &mut engine.world,
            GeologyBoot::Key(secret(KEY_1))
        ));
    }
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(store, &mut out, &mut fx)
        .expect("the world boots");
    let tw = TestWorld::from_engine(engine);
    no_refused_writes(&tw);
    tw
}

/// A booted world before any fire: keyed or not, offer keys and oracle installed.
fn booted(keyed: bool) -> Rig {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), keyed);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    Rig {
        tw,
        oracle,
        store: HashMap::new(),
        typed: BTreeMap::new(),
        keyed,
    }
}

/// Boot (keyed or not), settle and stock every port, at world rate 1.
fn stocked_with(keyed: bool) -> Rig {
    let mut rig = booted(keyed);
    sweep(&mut rig.tw, 120);
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    rig
}

/// The keyed rig.
fn stocked() -> Rig {
    stocked_with(true)
}

/// Fold every write since the first boot and reboot into the same roster.
fn reboot(rig: Rig) -> Rig {
    let Rig {
        mut tw,
        oracle,
        mut store,
        typed,
        keyed,
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
    let mut tw = boot_over(world, store.clone(), keyed);
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
        store,
        typed,
        keyed,
    }
}

/// Everything due `secs` on.
fn sweep(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    tw.seen_by(&events, who).join("\n")
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
    let mut bytes = [0x66_u8; 16];
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

/// Drive one queued submission the way the world task does, keeping its typed post-images; the
/// provider's outcome data on success, the prepare's own message on refusal.
fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<serde_json::Value, String> {
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
    let (prepared, authoritative) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes(token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let outcome = authoritative.outcome().data().clone();
    let (ops, install) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .map_err(|error| format!("{error:?}"))?;
    keep(rig, ops);
    Ok(outcome)
}

/// Every queued submission of the port's, inventory's, market's and industry's lanes; asserts
/// none kept of tacsim's is GOD's.
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from a tick");
        }
        if ["tacsim", "inventory", "market", "industry"].contains(&owner) {
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

/// One fire (6 s on), its submissions drained (not landed).
fn fire(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    drain(&mut rig.tw)
}

/// Fire and land the port tick until every port is stocked and listed (a site still opening is
/// the site clerk's work, not the stock clerks').
fn stock(rig: &mut Rig) {
    let clerks = site_clerks(&rig.tw);
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
            })
            .collect();
        if gated.len() > 1 && gated.iter().all(|line| line.ends_with("[ready]")) {
            return;
        }
        // The site clerks' acts are left for the tests that open their sites.
        let queued: Vec<AuthoritativeSubmit> = fire(rig)
            .into_iter()
            .filter(|submit| !clerks.contains(&submit.actor))
            .collect();
        land_all(rig, &queued);
    }
    panic!("the ports were not stocked within 40 fires");
}

/// The six site clerks.
fn site_clerks(tw: &TestWorld) -> Vec<ObjId> {
    SITE_PORTS
        .iter()
        .filter_map(
            |entry| match store(tw).port_map(&cid(LANE_PACK), &cid(entry)) {
                PortMapState::Present(map) => map.site.as_ref().and_then(|row| row.clerk),
                _ => None,
            },
        )
        .map(ObjId)
        .collect()
}

fn set_rate(tw: &mut TestWorld, rate: u32) {
    if tw.world().world_timeline().current().rate != rate {
        let out = run(tw, GOD, &format!("@tacsim/time rate={rate}"));
        assert_eq!(tw.world().world_timeline().current().rate, rate, "{out}");
    }
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

/// Open every lane site, keeping the typed post-images; the port fires each took.
fn open_all(rig: &mut Rig) -> BTreeMap<String, u32> {
    let mut kept = Vec::new();
    let fires = open_sites_keeping(&mut rig.tw, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    fires
}

/// Open one site, keeping the typed post-images; the port fires it took.
fn open_one(rig: &mut Rig, entry: &str) -> u32 {
    let mut kept = Vec::new();
    let fires = open_site_keeping(&mut rig.tw, entry, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    fires
}

// ---------------------------------------------------------------------------
// Readers
// ---------------------------------------------------------------------------

fn industry(tw: &TestWorld) -> IndustryFamilies {
    tw.world()
        .module_state::<IndustryHandles>()
        .expect("industry handles")
        .0
        .clone()
}

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
}

fn deposit(tw: &mut TestWorld, id: OreDepositId) -> Option<OreDeposit> {
    let families = industry(tw);
    typed(tw, &families.deposits, &id)
}

/// Every deposit on the world, by id.
fn all_deposits(tw: &TestWorld) -> BTreeMap<OreDepositId, OreDeposit> {
    let families = industry(tw);
    let state = tw.engine().action_state();
    let mut out = BTreeMap::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = state
            .scan_family(&families.deposits, cursor.as_deref(), 32)
            .expect("the deposits read");
        for entry in page.entries() {
            out.insert(entry.value().id(), entry.value().clone());
        }
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    out
}

fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    families
        .pool(&mut txn, EndpointRef(vault), &cid(good))
        .expect("the pool reads")
        .map_or(0, |pool| pool.quantity)
}

fn cursor(tw: &mut TestWorld, id: OreDepositId) -> Option<PocketCursor> {
    let pockets = handles(tw).families.pockets;
    typed(tw, &pockets, &id)
}

fn spoil(tw: &TestWorld, entry: &str) -> VaultId {
    VaultId(
        port_map(tw, entry)
            .site
            .and_then(|row| row.spoil_vault)
            .expect("the spoil vault"),
    )
}

fn site_window(location: &str) -> mush_atlas::window::WindowFrame {
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

/// A lane site's spec, body, ground and site definition, as a test reads them.
fn ground_of(
    tw: &mut TestWorld,
    entry: &str,
) -> (
    SiteGeologySpec,
    Arc<BodyGeology>,
    Arc<SiteGround>,
    tacsim::port::SiteDef,
) {
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
    let site = def.site.clone().expect("a site");
    let spec = tacsim::port::site::geology_spec(&site, &location, &body).expect("a spec");
    let ground = site_ground_sync(tw.world_mut(), &site_window(def.location.as_str()), &spec)
        .expect("the site builds");
    (spec, body, ground, site)
}

/// A site's opening plan, as the clerk reads it.
fn plan_of(tw: &mut TestWorld, entry: &str) -> Vec<OpeningRow> {
    let (_, _, ground, site) = ground_of(tw, entry);
    let row = port_map(tw, entry).site.expect("a site row");
    opening_plan(&cid(entry), &site, &row, &ground.geology).expect("the plan reads")
}

/// The Registry's org principal (the sites' issuer).
fn registry(tw: &TestWorld) -> mush_org::PrincipalId {
    mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(tw, "ceres_port").orgs[&cid("freeport_registry")].org,
    ))
}

// ---------------------------------------------------------------------------
// D1 (A1)
// ---------------------------------------------------------------------------

/// **Two keyed worlds with one key open identical reserves** (A1): every lane site opened on
/// two boots with `KEY_1`; for every site and every plan row the deposit records are equal —
/// id, place, endpoint, commodity, initial reserve, `external` and jurisdiction. The real
/// tables open fewer than 64 deposits a site (D14: only the ore-bearing rows); each site's
/// count is printed and pinned (re-blessable), with the port fires each site took.
#[test]
fn sm6_two_boots_with_one_key_open_identical_reserves() {
    let mut a = stocked();
    let fires = open_all(&mut a);
    let mut b = stocked();
    let _ = open_all(&mut b);
    let registry = registry(&a.tw);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in SITE_PORTS {
        let plan = plan_of(&mut a.tw, entry);
        counts.insert(entry.to_owned(), plan.len());
        for row in &plan {
            let one = deposit(&mut a.tw, row.deposit).expect("the deposit is open on world a");
            let two = deposit(&mut b.tw, row.deposit).expect("the deposit is open on world b");
            assert_eq!(one, two, "{}", row.receipt);
            assert_eq!(one.initial_reserve(), row.gross_t);
            assert_eq!(one.place(), row.place);
            assert_eq!(one.endpoint(), EndpointRef(row.vault));
            assert_eq!(one.commodity(), &row.good);
            assert_eq!(one.external().map(SmolStr::as_str), Some("tacsim.surface"));
            assert_eq!(one.jurisdiction(), Some(&registry));
            assert_eq!(one.accessibility().get(), 100);
        }
    }
    println!("sm6: real deposit counts per site {counts:?}; port fires to open {fires:?}");
    // Measured on the shipped tables with `KEY_1` (re-blessable: a table or key change moves
    // them); each under 64 wherever a family is barren (D14) — Ceres's strategic elements are
    // barren in 950 ‰ of its provinces (Kerwan Floor's is not), Vesta's carbon feedstock in
    // all. The port fires measured with them: 49 for each unworked 48-deposit site, 65 for
    // Kerwan, 97 for the worked Commons (48 opens and 48 transfers), one fire of warm each.
    let pinned: BTreeMap<String, usize> = [
        ("ahuna_port", 48),
        ("commons_port", 48),
        ("juling_port", 48),
        ("kerwan_port", 64),
        ("occator_port", 48),
        ("rheasilvia_port", 48),
    ]
    .into_iter()
    .map(|(entry, n)| (entry.to_owned(), n))
    .collect();
    assert_eq!(counts, pinned, "the real sites' deposit counts");
    assert_eq!(all_deposits(&a.tw).len(), counts.values().sum::<usize>());
    no_refused_writes(&a.tw);
}

// ---------------------------------------------------------------------------
// D2 (A2)
// ---------------------------------------------------------------------------

/// The block of letter `letter`: `(bx, by)` from the south-west.
fn block_of(letter: char) -> (i64, i64) {
    let at = tacsim::port::BLOCK_LETTERS
        .iter()
        .position(|l| *l == letter)
        .expect("a letter");
    let at = i64::try_from(at).expect("small");
    (at % 4, 3 - at / 4)
}

/// **The truth of one block's family**, computed independently of the plan: `field_ground` at
/// each of the block's four body-lattice fields under the block's shift (as
/// `chart::field_ground_at` computes it), summing its ore pockets' tonnes — and, at the block's
/// authored `permille`, the worked share written out pocket by pocket from §3.1 and D6 (review
/// T2): each ore pocket gives `⌊t × ‰ / 1000⌋`, kept when ≥ 1, keyed by its block pocket number
/// `(15 − py % 16)·16 + px % 16 + 1` (§3.1's address rule), in pocket-number order.
fn block_truth(
    spec: &SiteGeologySpec,
    body: &BodyGeology,
    letter: char,
    family: OreFamily,
    permille: u64,
) -> (u64, Vec<(u16, u32)>) {
    let (bx, by) = block_of(letter);
    let shift = spec.shifts[usize::try_from(by * 4 + bx).expect("a block")];
    let slot = usize::from(family.code());
    let mut sum = 0_u64;
    let mut worked = Vec::new();
    for fy in (2 * by)..(2 * by + 2) {
        for fx in (2 * bx)..(2 * bx + 2) {
            let field = field_ground(
                &mac(KEY_1),
                body,
                (spec.origin.0 / 8 + fx, spec.origin.1 / 8 + fy),
                shift,
            )
            .expect("the field's ground");
            for ly in 0..8 {
                for lx in 0..8 {
                    let row = field.pocket(lx, ly).expect("a pocket");
                    if row.classes[slot] != Word::Barren {
                        let tonnes = u64::from(row.tonnes[slot]);
                        sum += tonnes;
                        let share = tonnes * permille / 1_000;
                        if share >= 1 {
                            let (px, py) = (fx * 8 + lx, fy * 8 + ly);
                            let number = (15 - py % 16) * 16 + px % 16 + 1;
                            worked.push((
                                u16::try_from(number).expect("a pocket number"),
                                u32::try_from(share).expect("a share"),
                            ));
                        }
                    }
                }
            }
        }
    }
    worked.sort_unstable();
    (sum, worked)
}

/// **Each block holds its pockets, and the spoil its worked share** (A2; lead ruling 1, D6):
/// on Ahuna Flank and Freeport Commons every deposit's initial reserve equals the sum of its
/// block's ore pockets of its family, read independently through the chart's field ground,
/// and equals the plan's gross; the site's gross per family is the site table's totals; the
/// Commons's spoil holds exactly the worked share per ore good and each deposit pool the gross
/// less it; and one industry sweep later nothing is quarantined.
#[test]
fn sm6_each_block_holds_its_pockets_and_the_spoil_its_worked_share() {
    let mut rig = stocked();
    let _ = open_all(&mut rig);
    for entry in [AHUNA, COMMONS] {
        let (spec, body, ground, site) = ground_of(&mut rig.tw, entry);
        let plan = plan_of(&mut rig.tw, entry);
        let mut gross = [0_u64; 4];
        let mut worked: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        let mut unworked_rows = 0_usize;
        for row in &plan {
            let permille = u64::from(site.block(row.letter).worked_permille);
            let (truth, shares) = block_truth(&spec, &body, row.letter, row.family, permille);
            let open = deposit(&mut rig.tw, row.deposit).expect("open");
            assert_eq!(open.initial_reserve(), truth, "{}", row.receipt);
            assert_eq!(row.gross_t, truth, "{}", row.receipt);
            // The worked share, per pocket, against the independent truth (D6; review T2): the
            // plan's cursor, its worked total, and (below) the stored cursor and the pools.
            let planned: Vec<(u16, u32)> = row
                .cursor
                .iter()
                .map(|kept| (kept.pocket, kept.drawn_t))
                .collect();
            assert_eq!(planned, shares, "{}'s cursor", row.receipt);
            assert_eq!(
                row.worked_t,
                shares.iter().map(|(_, t)| u64::from(*t)).sum::<u64>(),
                "{}",
                row.receipt
            );
            if permille == 0 {
                assert!(
                    row.cursor.is_empty(),
                    "{}: 0 permille draws nothing",
                    row.receipt
                );
                unworked_rows += 1;
            }
            gross[usize::from(row.family.code())] += row.gross_t;
            *worked.entry(row.good.clone()).or_default() += row.worked_t;
            assert_eq!(
                pool(&mut rig.tw, row.vault, row.good.as_str()),
                row.gross_t - row.worked_t,
                "{}'s pool",
                row.receipt
            );
            if row.worked_t > 0 {
                let kept = cursor(&mut rig.tw, row.deposit).expect("a cursor");
                assert_eq!(kept.drawn_total(), Some(row.worked_t));
                assert_eq!(kept.rows, row.cursor);
                assert!(kept.rows.iter().all(|kept| kept.lock.is_none()));
            } else {
                assert!(
                    cursor(&mut rig.tw, row.deposit).is_none(),
                    "{}",
                    row.receipt
                );
            }
        }
        let totals = ground.geology.totals();
        for family in OreFamily::ALL {
            let slot = usize::from(family.code());
            assert_eq!(
                gross[slot],
                u64::from(totals[slot].1),
                "{entry}: {}",
                family.id()
            );
        }
        let spoil = spoil(&rig.tw, entry);
        for (good, share) in &worked {
            assert_eq!(
                pool(&mut rig.tw, spoil, good.as_str()),
                *share,
                "{entry}: {good}"
            );
        }
        if entry == COMMONS {
            assert!(
                worked.values().any(|share| *share > 0),
                "the commons is worked"
            );
        } else {
            assert!(worked.values().all(|share| *share == 0), "Ahuna is not");
            assert_eq!(
                unworked_rows,
                plan.len(),
                "every Ahuna block is at 0 permille"
            );
        }
    }
    // One industry sweep later: nothing quarantined.
    sweep(&mut rig.tw, 61);
    let quarantined = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .expect("the industry store")
        .quarantined();
    assert!(quarantined.is_empty(), "{quarantined:?}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D3 (A3)
// ---------------------------------------------------------------------------

/// What a site's opening left on the world.
#[derive(Debug, PartialEq, Eq)]
struct Census {
    deposits: BTreeMap<OreDepositId, OreDeposit>,
    pools: BTreeMap<OreDepositId, u64>,
    spoils: BTreeMap<(String, String), u64>,
    cursors: BTreeMap<OreDepositId, Option<PocketCursor>>,
    vaults: usize,
}

fn census(tw: &mut TestWorld) -> Census {
    let deposits = all_deposits(tw);
    let mut pools = BTreeMap::new();
    let mut cursors = BTreeMap::new();
    for (id, deposit) in &deposits {
        pools.insert(
            *id,
            pool(tw, deposit.endpoint().vault(), deposit.commodity().as_str()),
        );
        cursors.insert(*id, cursor(tw, *id));
    }
    let mut spoils = BTreeMap::new();
    for entry in SITE_PORTS {
        let vault = spoil(tw, entry);
        for good in ["metal_ore", "ice_ore", "carbon_ore", "strategic_ore"] {
            spoils.insert((entry.to_owned(), good.to_owned()), pool(tw, vault, good));
        }
    }
    let vaults = tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .vaults()
        .count();
    Census {
        deposits,
        pools,
        spoils,
        cursors,
        vaults,
    }
}

/// **The applier submits nothing, and nothing opens twice** (A3; lead ruling 2): after the
/// lane applies and before any port fire no deposit exists and no site clerk's submission is
/// queued; once every site is open, the real lay-up pass over every site entry
/// (`PortApplier::update`, SM.3b test 12's path), a reboot and twenty more port fires leave the
/// deposits, their pools, the spoils, the cursors and the vault census exactly as they were,
/// and no site clerk submits again.
#[test]
fn sm6_the_applier_submits_nothing_and_nothing_opens_twice() {
    let mut rig = booted(true);
    assert!(
        all_deposits(&rig.tw).is_empty(),
        "the applier opens nothing"
    );
    let clerks = site_clerks(&rig.tw);
    assert_eq!(clerks.len(), 6);
    let queued = drain(&mut rig.tw);
    assert!(
        queued.iter().all(|submit| !clerks.contains(&submit.actor)),
        "the applier and the boot pass submit nothing for a clerk"
    );
    sweep(&mut rig.tw, 120);
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    let _ = open_all(&mut rig);
    let before = census(&mut rig.tw);
    assert!(!before.deposits.is_empty());
    // The real lay-up pass, run again over every site entry.
    let pack = mush_content::load_pack(LANE_JSON).expect("the lane verifies");
    let placed = rig
        .tw
        .world()
        .module_state::<mush_content::ContentState>()
        .and_then(|state| state.latches.applied(&cid(LANE_PACK)))
        .map(mush_content::Applied::placed)
        .expect("the lane is applied");
    for entry in SITE_PORTS {
        let id = cid(entry);
        let raw = pack.entry(&id).cloned().expect("a site entry");
        let applied = support::with_ctx(&mut rig.tw, |ctx| {
            mush_content::EntryApplier::update(
                &tacsim::port::apply::PortApplier,
                ctx,
                &pack,
                &id,
                &raw,
                None,
                &placed,
            )
        });
        assert!(applied.is_ok(), "{entry}: {applied:?}");
    }
    assert!(
        drain(&mut rig.tw)
            .iter()
            .all(|s| !clerks.contains(&s.actor))
    );
    assert_eq!(census(&mut rig.tw), before, "the lay-up pass opens nothing");
    // A reboot, then twenty port fires: the sites read open again and nobody submits.
    let mut rig = reboot(rig);
    let mut landed = 0_u32;
    for _ in 0..20 {
        landed += site_fire(&mut rig.tw, &mut |_| {});
    }
    assert_eq!(landed, 0, "no site clerk submits after its site is open");
    for entry in SITE_PORTS {
        assert_eq!(
            site_line(&rig.tw, entry).map(|(state, _)| state),
            Some(LineState::Ready),
            "{entry} reads open again"
        );
    }
    assert_eq!(census(&mut rig.tw), before, "nothing opened twice");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D4 (A4)
// ---------------------------------------------------------------------------

/// Every family rich in every province, field and pocket (test data).
const ALL_RICH: &str = r#"{
  "province": {
    "metals": { "rich": 1000 }, "volatiles": { "rich": 1000 },
    "carbon_feedstock": { "rich": 1000 }, "strategic_elements": { "rich": 1000 }
  },
  "field": {
    "barren": { "barren": 1000 }, "lean": { "lean": 1000 },
    "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  },
  "pocket": {
    "lean": { "lean": 1000 }, "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  }
}"#;

/// **A site opens sixty-four deposits over sixteen places** (A4; §13's SM.6 row, met by an
/// all-families table, D14): Ceres's table replaced by one where every family is rich
/// everywhere, Ahuna Flank opens 64 deposits, four at each block place (industry's place
/// census), in at most 64 + its worked rows (none) + 4 port fires; a ninth deposit on one place
/// refuses at industry (`MAX_ORE_DEPOSITS_PER_PLACE`).
#[test]
fn sm6_a_site_opens_sixty_four_deposits_over_sixteen_places() {
    let mut rig = stocked();
    let wire: GeologyTableWire = serde_json::from_str(ALL_RICH).expect("the table parses");
    let table = GeologyTable::from_wire(&wire).expect("the table checks");
    tacsim::surface::install_body_geology(
        rig.tw.world_mut(),
        BodyGeology::new(cid("ceres"), geology_keyed::CERES_RADIUS_KM, table).expect("ceres"),
    )
    .expect("installed");
    let fires = open_one(&mut rig, AHUNA);
    let plan = plan_of(&mut rig.tw, AHUNA);
    assert_eq!(plan.len(), 64, "four families in each of sixteen blocks");
    let worked = plan.iter().filter(|row| row.worked_t > 0).count();
    assert!(
        usize::try_from(fires).expect("small") <= 64 + worked + 4,
        "{fires} port fires"
    );
    println!("sm6: Ahuna Flank, every family rich, opened in {fires} port fires");
    let families = industry(&rig.tw);
    let row = port_map(&rig.tw, AHUNA).site.expect("a row");
    for (letter, place) in &row.blocks {
        let census = typed(&mut rig.tw, &families.place_census, &ObjId(*place))
            .map(mush_industry::PlaceCensus::get);
        assert_eq!(census, Some(4), "block {letter}");
    }
    let mut per_letter: BTreeMap<char, usize> = BTreeMap::new();
    for row in &plan {
        *per_letter.entry(row.letter).or_default() += 1;
        assert!(deposit(&mut rig.tw, row.deposit).is_some());
    }
    assert!(per_letter.values().all(|n| *n == 4));
    // Four more by staff's hand on block A reach industry's cap of eight; the ninth refuses.
    let place = ObjId(row.blocks["A"]);
    let open = |tw: &mut TestWorld, n: u64| {
        let body = OreDeposit::new(
            OreDepositId(0x5600 + n),
            place,
            cid("metal_ore"),
            Grade::new(0).expect("grade"),
            mush_industry::Accessibility::new(100).expect("accessibility"),
            mush_industry::BandId::new("surface").expect("band"),
            mush_industry::BandId::new("surface").expect("band"),
            mush_industry::YieldCurve::new(vec![mush_industry::CurvePoint {
                cumulative: 0,
                per_cycle: 1,
            }])
            .expect("curve"),
            EndpointRef(VaultId(0x5600 + n)),
            10,
        )
        .expect("a body");
        let result = {
            let mut txn = tw
                .engine_mut()
                .action_transaction(&families.deposits)
                .expect("an industry transaction");
            families
                .open_deposit(&mut txn, body)
                .map(|()| txn.prepare().expect("prepares"))
        };
        result.map(|prepared| {
            tw.engine_mut()
                .install_prepared_action(prepared.into_parts().1)
                .expect("installs");
        })
    };
    for n in 0..4 {
        open(&mut rig.tw, n).expect("under the cap");
    }
    assert!(
        matches!(
            open(&mut rig.tw, 4),
            Err(mush_industry::IndustryError::Record(
                mush_industry::IndustryRecordError::PlaceCensus { .. }
            ))
        ),
        "the ninth on one place refuses"
    );
}

// ---------------------------------------------------------------------------
// D5 (A5) and D6 (A6)
// ---------------------------------------------------------------------------

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

/// A chartered, licensed pilot whose hull the yard has made a Dray-P.
fn prospector(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let pilot = pilot(rig, who, seed);
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

/// `tank` t and `locker` crew-days into the hold.
fn load(rig: &mut Rig, row: &HullRow, tank: u64, locker: u64, lot: u64) {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    let vault = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id;
    seed_pool(rig, vault, "propellant", tank, LotId(lot));
    seed_pool(rig, vault, "provisions", locker, LotId(lot + 1));
}

/// Board from C01.
fn board(rig: &mut Rig, pilot: ObjId) {
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
}

/// A licensed pilot aboard a loaded Dray-P at Ceres.
fn aboard(rig: &mut Rig) -> ObjId {
    let (pilot, row) = prospector(rig, "Pilot", 1, "Salt Moth");
    load(rig, &row, 20, 20, 9_100);
    board(rig, pilot);
    pilot
}

/// **`@extract` refuses a surface block** (A5; lead ruling 4; closes SM.5's residual F12
/// (ii)): a licensed pilot holding a staff-filed claim (`@claim/file`) over an Ahuna deposit
/// is never offered it — `authorized_deposits` lacks it — and `@extract/start` and
/// `@extract/cycle` on it land nothing: no `industry.run`, no lot, the pool exact.
#[test]
fn sm6_extract_refuses_a_surface_block() {
    let mut rig = stocked();
    let _ = open_one(&mut rig, AHUNA);
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let row = plan_of(&mut rig.tw, AHUNA)
        .into_iter()
        .next()
        .expect("Ahuna opens a deposit");
    let registry = registry(&rig.tw);
    let issuer = mush_economy::principal_token(&registry);
    // The term an IC day past now (the term is an absolute IC second): live through the test.
    let term = rig.tw.world().world_ic_now().at + 86_400;
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@claim/file deposit:{}=org:{}/obj:{}/exclusive/1/{term}/transferable",
            row.deposit,
            issuer.trim_start_matches("org:"),
            pilot.0
        ),
    );
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let families = industry(&rig.tw);
    let claims = {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&families.deposits)
            .expect("a transaction");
        families
            .claims_over(
                &mut txn,
                mush_industry::ClaimSubject::Deposit {
                    deposit: row.deposit,
                },
            )
            .expect("the claims read")
    };
    assert_eq!(claims.len(), 1, "the staff-filed claim stands");
    assert_eq!(claims[0].holder(), &mush_org::PrincipalId::Object(pilot));
    // The positive control (review T4): tenure, licence and body all answer for this pilot —
    // `extraction_authorized` is `Ok` — so `external` is the only thing that refuses below.
    let body = deposit(&mut rig.tw, row.deposit).expect("the deposit is open");
    let remaining = pool(&mut rig.tw, row.vault, row.good.as_str());
    let licence = mush_economy::InstrumentId::new(tacsim::economy::LICENCE_FREEPORT)
        .expect("the licence instrument");
    assert_eq!(
        mush_industry::extraction_authorized(
            rig.tw.world(),
            pilot,
            &mush_org::PrincipalId::Object(pilot),
            &body,
            remaining,
            &claims,
            Some(&licence),
            &rig.tw.world().world_ic_now(),
        )
        .map(|authorized| authorized.claim),
        Ok(claims[0].id()),
        "the pilot is authorized on everything but the surface rule"
    );
    let offered = mush_industry::provider::authorized_deposits(
        rig.tw.engine(),
        &families,
        pilot,
        &mush_org::PrincipalId::Object(pilot),
    );
    assert!(!offered.contains(&row.deposit), "never offered to @extract");
    let before = pool(&mut rig.tw, row.vault, row.good.as_str());
    for line in [
        format!("@extract/start {}={}", row.deposit, row.vault.0),
        format!("@extract/cycle {}", row.deposit),
    ] {
        let _ = run(&mut rig.tw, pilot, &line);
        for submit in drain(&mut rig.tw) {
            assert!(land(&mut rig, &submit).is_err(), "{line} lands nothing");
        }
    }
    assert!(
        typed(&mut rig.tw, &families.runs, &row.deposit).is_none(),
        "no run written"
    );
    assert_eq!(
        pool(&mut rig.tw, row.vault, row.good.as_str()),
        before,
        "no lot minted"
    );
    no_refused_writes(&rig.tw);
}

/// **A forged debit quarantines its block and no other** (A6; D11): one tonne debited from one
/// Ahuna deposit's pool by a direct inventory transaction (`seed_pool`'s shape); one industry
/// sweep later every surface deposit at that block place is quarantined `DrawsMismatch`, and
/// no other block's is.
#[test]
fn sm6_a_forged_debit_quarantines_its_block_and_no_other() {
    let mut rig = stocked();
    let _ = open_one(&mut rig, AHUNA);
    let plan = plan_of(&mut rig.tw, AHUNA);
    let forged = plan.first().expect("Ahuna opens a deposit").clone();
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let prepared = {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        families
            .debit(&mut txn, EndpointRef(forged.vault), &forged.good, 1)
            .expect("the forged debit");
        txn.prepare().expect("prepares")
    };
    rig.tw
        .engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .expect("installs");
    sweep(&mut rig.tw, 61);
    let store = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .expect("the industry store");
    let mut closed = 0;
    for row in &plan {
        if row.place == forged.place {
            assert_eq!(
                store.quarantine(row.deposit),
                Some(Quarantine::DrawsMismatch),
                "{}",
                row.receipt
            );
            closed += 1;
        } else {
            assert_eq!(store.quarantine(row.deposit), None, "{}", row.receipt);
        }
    }
    assert!(closed >= 1);
}

// ---------------------------------------------------------------------------
// D7 (A13) and D8
// ---------------------------------------------------------------------------

/// **The site reads "opening, N of M" and its pad is closed until it is open** (A13; D2):
/// mid-opening the Site line reads `opening, N of M` with `0 < N < M` and `+ship/plan ahuna`
/// from Ceres answers `SITE_CLOSED`; once open the line reads `{M} deposits open.` and the plan
/// files; a cold reboot reads "still being mapped" until the surface tick warms the site and
/// the port tick confirms it.
#[test]
fn sm6_the_site_reads_opening_and_its_pad_is_closed_until_open() {
    let mut rig = stocked();
    let pilot = aboard(&mut rig);
    let closed = SITE_CLOSED.replace("{site}", "Ahuna Flank");
    let laid = "ahuna_port site: 16 block places, the site clerk, the spoil and 16 block vaults \
                laid up";
    let mut mid = None;
    for _ in 0..200 {
        let (state, text) = site_line(&rig.tw, AHUNA).expect("the line");
        if let Some(rest) = text.strip_prefix(&format!("{laid}; opening, ")) {
            let (n, m) = rest
                .trim_end_matches('.')
                .split_once(" of ")
                .expect("N of M");
            let (n, m): (u32, u32) = (n.parse().expect("N"), m.parse().expect("M"));
            if n > 0 {
                assert!(n < m, "{text}");
                assert_eq!(state, LineState::Missing);
                mid = Some(m);
                break;
            }
        }
        let mut kept = Vec::new();
        let _ = site_fire(&mut rig.tw, &mut |ops| kept.push(ops));
        for ops in kept {
            keep(&mut rig, ops);
        }
    }
    let total = mid.expect("the site was seen mid-opening");
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    assert_eq!(
        plan.lines().nth(1),
        Some(closed.as_str()),
        "the pad is closed while it opens: {plan}"
    );
    let _ = open_one(&mut rig, AHUNA);
    assert_eq!(
        site_line(&rig.tw, AHUNA),
        Some((LineState::Ready, format!("{laid}; {total} deposits open.")))
    );
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    assert!(!plan.contains(&closed), "{plan}");
    assert!(plan.starts_with("Plan for Ahuna Flank:"), "{plan}");
    // A cold reboot: still being mapped, until a warm and a confirm.
    let mut rig = reboot(rig);
    assert_eq!(
        site_line(&rig.tw, AHUNA),
        Some((
            LineState::Missing,
            format!("{laid}; opening: its ground is still being mapped.")
        ))
    );
    let mut fires = 0;
    while site_line(&rig.tw, AHUNA).map(|(state, _)| state) != Some(LineState::Ready) {
        assert!(fires < 4, "{:?}", site_line(&rig.tw, AHUNA));
        let landed = site_fire(&mut rig.tw, &mut |_| {});
        assert_eq!(
            landed, 0,
            "an open site's clerk submits nothing after a reboot"
        );
        fires += 1;
    }
    assert_eq!(
        site_line(&rig.tw, AHUNA),
        Some((LineState::Ready, format!("{laid}; {total} deposits open.")))
    );
    no_refused_writes(&rig.tw);
}

/// **A dark layer opens nothing and keeps the pad open** (D2; lead fork (b)): in an unkeyed
/// world every Site line reads the dark text and Ready, the site clerks submit nothing over
/// twenty fires, and `+ship/plan ahuna` plans.
#[test]
fn sm6_a_dark_layer_opens_nothing_and_keeps_the_pad_open() {
    let mut rig = stocked_with(false);
    for entry in SITE_PORTS {
        assert_eq!(
            site_line(&rig.tw, entry),
            Some((
                LineState::Ready,
                format!(
                    "{entry} site: 16 block places, the site clerk, the spoil and 16 block \
                     vaults laid up; no deposits while the geology layer is dark."
                )
            ))
        );
    }
    let mut landed = 0;
    for _ in 0..20 {
        landed += site_fire(&mut rig.tw, &mut |_| {});
    }
    assert_eq!(landed, 0, "a dark layer's clerks submit nothing");
    assert!(all_deposits(&rig.tw).is_empty());
    let pilot = aboard(&mut rig);
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    assert!(plan.starts_with("Plan for Ahuna Flank:"), "{plan}");
    assert!(
        !plan.contains(&SITE_CLOSED.replace("{site}", "Ahuna Flank")),
        "{plan}"
    );
}

// ---------------------------------------------------------------------------
// The work act's guards and the mismatch hold (review T6), the issuer (review F3)
// ---------------------------------------------------------------------------

/// A site's clerk.
fn clerk_of(tw: &TestWorld, entry: &str) -> ObjId {
    ObjId(
        port_map(tw, entry)
            .site
            .and_then(|row| row.clerk)
            .expect("a site clerk"),
    )
}

/// One `tacsim_site_work` row as its offer carries it (`work_row_value`'s shape), naming
/// `clerk`.
fn work_rows(tw: &TestWorld, entry: &str, row: &OpeningRow, clerk: ObjId) -> serde_json::Value {
    let mut rows = serde_json::Map::new();
    rows.insert(
        row.receipt.clone(),
        serde_json::json!({
            "deposit": tacsim::surface::cursor::cursor_key_text(&row.deposit),
            "commodity": row.good.as_str(),
            "from": row.vault.0,
            "to": spoil(tw, entry).0,
            "worked_t": row.worked_t,
            "cursor": row.cursor,
            "clerk": clerk.0,
        }),
    );
    serde_json::Value::Object(rows)
}

/// A hand-built `tacsim_site_work` invocation by `actor` (one that never met the offer),
/// prepared; `Ok` installs it. The refusal's code and message otherwise.
fn hand_work(
    tw: &mut TestWorld,
    actor: ObjId,
    parameters: serde_json::Value,
) -> Result<(), (mush_core::ActionErrorCode, String)> {
    let action = handles(tw).site_work.expect("the site work is registered");
    let token = next_token();
    let invocation = mush_core::ActionInvocation::new(mush_core::ActionInvocationDraft {
        request: RequestUid::from_bytes(token),
        principal: mush_core::PrincipalId::Object { id: actor },
        actor,
        offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes(
            [token[3]; 32],
        )),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: mush_core::PrincipalId::Object { id: actor },
                owner: action.key().owner.clone(),
                namespace: CanonicalId::new(action.key().id.as_str()).expect("a namespace"),
            },
            1,
            IdempotencyKey::Request(RequestUid::from_bytes(token)),
        ),
    })
    .expect("an invocation");
    match tw.engine_mut().prepare_registered_authoritative_action(
        &action,
        &invocation,
        CommitToken::from_bytes(token),
    ) {
        Ok((prepared, _)) => {
            tw.engine_mut()
                .install_prepared_action(prepared.into_parts().1)
                .expect("installs");
            Ok(())
        }
        Err(mush_core::ActionProviderPrepareError::Provider { source, .. }) => {
            Err((source.code(), source.message().to_owned()))
        }
        Err(other) => panic!("expected a provider refusal, got {other}"),
    }
}

/// **The worked act's own guards** (lead ruling 2; review T6): once the Commons is open, a
/// replayed `tacsim_site_work` for a worked row — hand-built, never offered — refuses
/// `Conflict` "that block is already worked" (the cursor sidecar is the replay guard); one
/// whose row names another clerk refuses `Unauthorized`; one whose row names a deposit that is
/// not its receipt's refuses `Integrity`. Each leaves the deposit's pool, the spoil and the
/// cursor exactly as they were.
#[test]
fn sm6_the_site_work_refuses_a_replay_another_clerk_and_a_foreign_deposit() {
    let mut rig = stocked();
    let _ = open_one(&mut rig, COMMONS);
    let clerk = clerk_of(&rig.tw, COMMONS);
    let stranger = clerk_of(&rig.tw, AHUNA);
    let plan = plan_of(&mut rig.tw, COMMONS);
    let row = plan
        .iter()
        .find(|row| row.worked_t > 0)
        .expect("the commons is worked")
        .clone();
    let spoil_vault = spoil(&rig.tw, COMMONS);
    let snapshot = |tw: &mut TestWorld| {
        (
            pool(tw, row.vault, row.good.as_str()),
            pool(tw, spoil_vault, row.good.as_str()),
            cursor(tw, row.deposit),
        )
    };
    let before = snapshot(&mut rig.tw);
    assert_eq!(before.0, row.gross_t - row.worked_t);
    assert!(before.2.is_some(), "the row is worked");
    let parameters =
        |rows: serde_json::Value| serde_json::json!({ "receipt": row.receipt, "rows": rows });
    // The replay: the very row the offer carried, again.
    let rows = work_rows(&rig.tw, COMMONS, &row, clerk);
    let replay = hand_work(&mut rig.tw, clerk, parameters(rows));
    assert_eq!(
        replay,
        Err((
            mush_core::ActionErrorCode::Conflict,
            "that block is already worked".to_owned()
        ))
    );
    assert_eq!(snapshot(&mut rig.tw), before, "a replay moves nothing");
    // Another clerk's row.
    let rows = work_rows(&rig.tw, COMMONS, &row, stranger);
    let foreign = hand_work(&mut rig.tw, clerk, parameters(rows));
    assert_eq!(
        foreign.map_err(|(code, _)| code),
        Err(mush_core::ActionErrorCode::Unauthorized)
    );
    // A row whose deposit is not its receipt's.
    let other = plan
        .iter()
        .find(|other| other.deposit != row.deposit)
        .expect("a second row");
    let mut rows = work_rows(&rig.tw, COMMONS, &row, clerk);
    rows[row.receipt.as_str()]["deposit"] =
        serde_json::json!(tacsim::surface::cursor::cursor_key_text(&other.deposit));
    let forged = hand_work(&mut rig.tw, clerk, parameters(rows));
    assert_eq!(
        forged.map_err(|(code, _)| code),
        Err(mush_core::ActionErrorCode::Integrity)
    );
    assert_eq!(snapshot(&mut rig.tw), before, "no refusal moves anything");
    no_refused_writes(&rig.tw);
}

/// Whether `actor` is offered the action whose presentation verb is `verb`.
fn offered(tw: &TestWorld, actor: ObjId, verb: &str) -> bool {
    let query = tw
        .engine()
        .legal_action_query(
            mush_core::PrincipalId::Object { id: actor },
            actor,
            mush_core::PerceivedRef::SelfActor,
        )
        .expect("a query");
    tw.engine()
        .project_legal_actions(&query)
        .expect("the projection")
        .projected()
        .into_iter()
        .any(|offer| offer.presentation().verb == verb)
}

/// **A deposit opened under another table is never worked silently** (the brief's 3.12
/// mismatch hold; review T6): the Commons's first worked row opened by staff's hand at a
/// reserve one tonne past the plan's gross; the clerk is then never offered
/// `tacsim_site_work`, submits nothing for the Commons over twenty fires, and the Commons reads
/// `opening, 0 of M` (Missing — so its pad answers `SITE_CLOSED`, as
/// `sm6_the_site_reads_opening_and_its_pad_is_closed_until_open` shows for any opening site),
/// its pool untouched and no cursor written.
#[test]
fn sm6_a_deposit_opened_under_another_table_holds_its_site_closed() {
    let mut rig = stocked();
    let clerk = clerk_of(&rig.tw, COMMONS);
    let plan = plan_of(&mut rig.tw, COMMONS);
    let row = plan.first().expect("the commons opens").clone();
    assert!(row.worked_t > 0, "the commons's first row is worked");
    let families = industry(&rig.tw);
    let body = OreDeposit::new(
        row.deposit,
        row.place,
        row.good.clone(),
        Grade::new(0).expect("grade"),
        mush_industry::Accessibility::new(100).expect("accessibility"),
        mush_industry::BandId::new("surface").expect("band"),
        mush_industry::BandId::new("surface").expect("band"),
        mush_industry::YieldCurve::new(vec![mush_industry::CurvePoint {
            cumulative: 0,
            per_cycle: 1,
        }])
        .expect("curve"),
        EndpointRef(row.vault),
        row.gross_t + 1,
    )
    .expect("a body");
    let prepared = {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&families.deposits)
            .expect("an industry transaction");
        families
            .open_deposit(&mut txn, body)
            .expect("staff open the deposit");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .expect("installs");
    keep(&mut rig, ops);
    let clerks = site_clerks(&rig.tw);
    let mut submitted = 0;
    for _ in 0..20 {
        let queued = fire(&mut rig);
        submitted += queued.iter().filter(|submit| submit.actor == clerk).count();
        let others: Vec<AuthoritativeSubmit> = queued
            .into_iter()
            .filter(|submit| clerks.contains(&submit.actor) && submit.actor != clerk)
            .collect();
        land_all(&mut rig, &others);
        assert!(
            !offered(&rig.tw, clerk, "tacsim-site-work"),
            "the mismatched row is never offered"
        );
    }
    assert_eq!(submitted, 0, "the commons's clerk holds");
    let total = plan.len();
    assert_eq!(
        site_line(&rig.tw, COMMONS),
        Some((
            LineState::Missing,
            format!(
                "commons_port site: 16 block places, the site clerk, the spoil and 16 block \
                 vaults laid up; opening, 0 of {total}."
            )
        ))
    );
    assert_eq!(
        pool(&mut rig.tw, row.vault, row.good.as_str()),
        row.gross_t + 1
    );
    assert!(cursor(&mut rig.tw, row.deposit).is_none());
}

/// **An issuer that does not resolve opens nothing** (lead ruling 7; review F3): with the
/// Registry's org row gone from every lane port map (RAM only, the store's test hook), Ahuna's
/// clerk is offered no opening, submits nothing, opens no deposit and its Site line reads the
/// failed text (Missing: the pad stays closed) — never a deposit without its jurisdiction.
#[test]
fn sm6_a_site_whose_issuer_does_not_resolve_opens_nothing() {
    let mut rig = stocked();
    let clerk = clerk_of(&rig.tw, AHUNA);
    let plan = plan_of(&mut rig.tw, AHUNA);
    let dropped = rig
        .tw
        .world_mut()
        .module_state_mut::<TacsimStore>()
        .expect("tacsim state")
        .drop_lane_org_for_tests(&cid("freeport_registry"));
    assert!(dropped > 0, "the Registry's row was on the maps");
    let mut submitted = 0;
    for _ in 0..12 {
        submitted += fire(&mut rig)
            .iter()
            .filter(|submit| submit.actor == clerk)
            .count();
        assert!(
            !offered(&rig.tw, clerk, "deposit-open"),
            "no opening is offered"
        );
    }
    assert_eq!(submitted, 0, "the clerk submits nothing");
    for row in &plan {
        assert!(
            deposit(&mut rig.tw, row.deposit).is_none(),
            "{}",
            row.receipt
        );
    }
    assert_eq!(
        site_line(&rig.tw, AHUNA),
        Some((
            LineState::Missing,
            "ahuna_port site: 16 block places, the site clerk, the spoil and 16 block vaults \
             laid up; opening: its ground could not be mapped (see the log)."
                .to_owned()
        ))
    );
}

// ---------------------------------------------------------------------------
// D9
// ---------------------------------------------------------------------------

/// **The opening writes no truth anywhere a player reads** (D9; lead ruling 11; reviewer F6):
/// every outcome of the clerk's open (industry's) and work (tacsim's) names the actor, the
/// deposit and the receipt (and the open its summary) — never a reserve or a tonne; both survey
/// bands read `surface`; no player is offered either action.
#[test]
fn sm6_the_opening_writes_no_truth_anywhere_a_player_reads() {
    let mut rig = stocked();
    let pilot = aboard(&mut rig);
    let clerks = site_clerks(&rig.tw);
    let commons_clerk = ObjId(
        port_map(&rig.tw, COMMONS)
            .site
            .and_then(|row| row.clerk)
            .expect("a clerk"),
    );
    let mut opens = 0;
    let mut works = 0;
    for _ in 0..300 {
        if support::sites::site_open(&rig.tw, COMMONS) {
            break;
        }
        let queued: Vec<AuthoritativeSubmit> = fire(&mut rig)
            .into_iter()
            .filter(|submit| clerks.contains(&submit.actor))
            .collect();
        for submit in &queued {
            let outcome = land(&mut rig, submit).expect("the clerk's act lands");
            let keys: Vec<&str> = outcome
                .as_object()
                .expect("an object")
                .keys()
                .map(String::as_str)
                .collect();
            match submit.action.key().id.as_str() {
                "industry_deposit_open" => {
                    assert_eq!(keys, vec!["actor", "deposit", "receipt", "summary", "verb"]);
                    if submit.actor == commons_clerk {
                        opens += 1;
                    }
                }
                "tacsim_site_work" => {
                    assert_eq!(keys, vec!["actor", "deposit", "receipt"]);
                    if submit.actor == commons_clerk {
                        works += 1;
                    }
                }
                // Doc 20 cut SM.10: an open site's clerk seeds its held blocks and papers
                // (Occator Rim's); the act's outcome names no deposit, count or reserve.
                "industry_claim_tenure" => {
                    assert_eq!(keys, vec!["actor", "place", "receipt", "summary", "verb"]);
                }
                other => panic!("a site clerk submitted {other}"),
            }
        }
    }
    assert!(
        opens > 0 && works > 0,
        "the commons opened ({opens}) and worked ({works})"
    );
    for deposit in all_deposits(&rig.tw).values() {
        assert_eq!(deposit.grade_class().as_str(), "surface");
        assert_eq!(deposit.reserve_class().as_str(), "surface");
    }
    let query = rig
        .tw
        .engine()
        .legal_action_query(
            mush_core::PrincipalId::Object { id: pilot },
            pilot,
            mush_core::PerceivedRef::SelfActor,
        )
        .expect("a query");
    let verbs: Vec<String> = rig
        .tw
        .engine()
        .project_legal_actions(&query)
        .expect("the projection")
        .projected()
        .into_iter()
        .map(|offer| offer.presentation().verb.clone())
        .collect();
    assert!(
        !verbs.iter().any(|verb| verb == "deposit-open"),
        "{verbs:?}"
    );
    assert!(
        !verbs.iter().any(|verb| verb == "tacsim-site-work"),
        "{verbs:?}"
    );
}

// ---------------------------------------------------------------------------
// D10
// ---------------------------------------------------------------------------

/// Industry's writing seams: a call to one from tacsim is a write.
const INDUSTRY_SEAMS: [&str; 12] = [
    "open_deposit(",
    "file_claim(",
    "put_claim(",
    "put_run(",
    "set_accessibility(",
    "renew_claim(",
    "lapse_claim(",
    "take_over_claim(",
    "transfer_tenure(",
    // Doc 20 cut SM.10: the block-level tenure seams and the termed transfer — industry
    // tenure writes tacsim reaches only through the composed `industry_claim_tenure`.
    "tenure_lapse(",
    "tenure_hold(",
    "transfer_tenure_termed(",
];

/// The raw typed-state writes, by their receiver spellings in the tree.
const RAW_WRITES: [&str; 6] = [
    "txn.insert(",
    "txn.replace(",
    "txn.remove(",
    ".action_txn().insert(",
    ".action_txn().replace(",
    ".action_txn().remove(",
];

/// `text` with every brace-delimited `#[cfg(test)]` item cut out (`panic_budget`'s rule: the
/// attribute, then everything to its item's closing brace, or to its `;` when the item has no
/// body) and every `//` comment line dropped — the production source, newlines kept.
fn production_source(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "#[cfg(test)]".len()..];
        let open = after.find('{');
        let semi = after.find(';');
        let end = match (open, semi) {
            (Some(open), Some(semi)) if semi < open => semi + 1,
            (Some(open), _) => {
                let mut depth = 0_i64;
                let mut close = after.len();
                for (i, c) in after[open..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                close = open + i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                close
            }
            (None, Some(semi)) => semi + 1,
            (None, None) => after.len(),
        };
        rest = &after[end..];
    }
    out.push_str(rest);
    out.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `word` occurs in `text` as a whole identifier (so `mush_industry` is not
/// `industry`).
fn names_ident(text: &str, word: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

/// Every identifier a `let` pattern binds: `x`, `mut x`, `Some(x)`, `Ok(x)`, `Err(x)`, and the
/// plain names of a tuple or tuple-struct pattern.
fn pattern_names(pattern: &str) -> Vec<String> {
    let mut names = Vec::new();
    for token in pattern.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if token.is_empty()
            || matches!(token, "mut" | "ref" | "Some" | "Ok" | "Err" | "None" | "_")
            || token.starts_with(|c: char| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            continue;
        }
        names.push(token.to_owned());
    }
    names
}

/// **The D10 scanner over one file's text** (whitespace-joined, so a rustfmt'd statement spread
/// over lines reads as one): every industry-seam call, and every raw typed-state write whose
/// argument list names an identifier bound — by a `let`, `if let` or `while let` whose
/// right-hand side names `IndustryHandles`, `IndustryFamilies`, `IndustryHandleCell`,
/// `industry_handles` or an `industry` identifier — or the `industry` / `industry_handles`
/// identifiers themselves.
fn industry_writes(label: &str, text: &str) -> Vec<String> {
    let code: String = production_source(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut offenders = Vec::new();
    for seam in INDUSTRY_SEAMS {
        for (at, _) in code.match_indices(seam) {
            offenders.push(format!(
                "{label}: calls {seam} near `{}`",
                &code[at.saturating_sub(40)..at]
            ));
        }
    }
    let industry_source = |rhs: &str| {
        ["IndustryHandles", "IndustryFamilies", "IndustryHandleCell"]
            .iter()
            .any(|ty| rhs.contains(ty))
            || names_ident(rhs, "industry_handles")
            || names_ident(rhs, "industry")
    };
    let mut bound: Vec<String> = vec!["industry".to_owned(), "industry_handles".to_owned()];
    for (at, _) in code.match_indices("let ") {
        if at > 0
            && code[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let statement = &code[at + 4..];
        let Some(eq) = statement.find(" = ") else {
            continue;
        };
        let pattern = &statement[..eq];
        let rhs = &statement[eq + 3..];
        let conditional = code[..at].ends_with("if ") || code[..at].ends_with("while ");
        let end = if conditional {
            rhs.find(" {").unwrap_or(rhs.len())
        } else {
            let semi = rhs.find(';').unwrap_or(rhs.len());
            rhs.find(" else {").map_or(semi, |e| e.min(semi))
        };
        if industry_source(&rhs[..end]) {
            bound.extend(pattern_names(pattern));
        }
    }
    bound.sort();
    bound.dedup();
    for write in RAW_WRITES {
        for (at, _) in code.match_indices(write) {
            let args: String = code[at + write.len()..]
                .chars()
                .take(200)
                .filter(|c| !c.is_whitespace())
                .collect();
            let first = args.split(',').next().unwrap_or("");
            if bound.iter().any(|name| {
                first == format!("&{name}")
                    || first.starts_with(&format!("&{name}."))
                    || first.starts_with(&format!("&self.{name}."))
                    || first.starts_with(&format!("&{name}.0."))
            }) {
                offenders.push(format!("{label}: {write}{first}"));
            }
        }
    }
    offenders
}

/// **The D10 scanner catches what it claims to** (review T5): a write through a `let Some(..)`
/// binding, a rustfmt'd multi-line binding and write, a write in production code that follows
/// a `#[cfg(test)]` item, a direct seam call, and a write through a plain handle binding are
/// each flagged; a read, a write to a tacsim family, and a write inside a `#[cfg(test)]` module
/// are not.
#[test]
fn sm6_the_industry_write_tripwire_flags_a_planted_write() {
    let flagged = [
        "fn a(&self, txn: &mut T) { let Some(fam) = self.industry.get() else { return; };\n\
         txn.insert(&fam.deposits, &id, body); }",
        "fn b(ctx: &mut Ctx, txn: &mut T) {\n    let handles = ctx\n        .w\n        \
         .module_state::<mush_industry::IndustryHandles>()\n        .map(|h| h.0.clone());\n    \
         txn.replace(\n        &handles.0.claims,\n        &id,\n        claim,\n    );\n}",
        "#[cfg(test)]\nfn helper() {}\n\nfn c(txn: &mut T, industry: &IndustryFamilies) {\n    \
         txn.insert(&industry.deposits, &k, v);\n}",
        "fn d(families: &IndustryFamilies, txn: &mut T) { families.open_deposit(txn, body); }",
        "fn e(txn: &mut T, w: &World) { let deposits = w.module_state::<IndustryHandles>()\
         .map(|h| h.0.deposits.clone()); txn.remove(&deposits, &k); }",
    ];
    for (n, snippet) in flagged.iter().enumerate() {
        assert!(
            !industry_writes("planted", snippet).is_empty(),
            "planted write {n} is not flagged:\n{snippet}"
        );
    }
    let clean = [
        "fn f(ctx: &Ctx, txn: &mut T) { let fam = ctx.w.module_state::<IndustryHandles>(); \
         let x = txn.get(&fam.0.deposits, &k); }",
        "fn g(txn: &mut T) { let pockets = handles.families.pockets.clone(); \
         txn.insert(&pockets, &k, v); }",
        "#[cfg(test)]\nmod tests {\n    fn h(txn: &mut T, industry: &IndustryFamilies) {\n        \
         txn.insert(&industry.deposits, &k, v);\n    }\n}",
        "fn i() { let action = mush_industry::action_ref(reg, verb); txn.insert(&action, &k, v); }",
    ];
    for (n, snippet) in clean.iter().enumerate() {
        assert_eq!(
            industry_writes("clean", snippet),
            Vec::<String>::new(),
            "clean snippet {n} is flagged:\n{snippet}"
        );
    }
}

/// **Tacsim never writes an industry family** (D10; reviewer F11; review T5): the typed-state
/// delegation of industry's families to tacsim (D1) grants write as well as read, so this source
/// tripwire holds tacsim to reads. Over the production source of every file under
/// `crates/tacsim/src` (`#[cfg(test)]` items cut out brace by brace, comment lines dropped,
/// whitespace joined so a statement spread over lines reads as one), [`industry_writes`] flags
/// any call of one of industry's writing seams, and any raw `insert(`/`replace(`/`remove(` whose
/// handle argument names an identifier bound from an industry handle (`let`, `let Some(..)`,
/// tuple patterns, `if let`, `while let`); `sm6_the_industry_write_tripwire_flags_a_planted_write`
/// proves it on planted snippets. **What it does not catch**: a handle passed into a helper
/// under another name, a binding whose right-hand side names no industry source (a field read
/// off an unnamed struct), braces inside string or char literals (which can misplace a
/// `#[cfg(test)]` item's end), or a macro-built call — it is a tripwire, not a proof.
#[test]
fn sm6_tacsim_never_writes_an_industry_family() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let mut offenders: Vec<String> = Vec::new();
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a source dir") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            files += 1;
            let text = std::fs::read_to_string(&path).expect("a source file");
            offenders.extend(industry_writes(&path.display().to_string(), &text));
        }
    }
    assert!(files > 50, "the tripwire read the crate ({files} files)");
    assert!(
        offenders.is_empty(),
        "tacsim writes an industry family:\n{}",
        offenders.join("\n")
    );
}
