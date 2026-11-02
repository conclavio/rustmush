//! **Doc 20 cut SM.12c-lane — the cost lane** (doc 20 §11's Tick, Caps and Instruments rows,
//! §13's SM.12 row and every Inherits clause of its acceptance cell, §16; doc 20a §5, the plan of
//! record; the train's last cut). What the surface-mining layer costs, measured before any site
//! goes live, and the three things doc 20 left to the figures decided on them.
//!
//! The binary holds:
//! - **the write census** (C1, `sm12c_every_act_kind_counts_its_writes_*`): every act kind of the
//!   mining layer — the surface starts and finishes, the plan's words and steps, the chart's
//!   forget and the preference document, tenure, the site clerk's acts, the licence and the lease,
//!   the mill's batch and restock, the dispatcher's delivery and survey — built at its structural
//!   worst case on the shipped lane through the real verbs and the real ticks, counted (a bundle's
//!   staged ops; a normal-lane act's module ops) and held to `cohort::WRITE_GATES`;
//! - **the surface tick's release gate** (C2, `sm12c_the_surface_tick_is_measured`): ten TVs on the
//!   shipped lane, 1,800 one-second fires, five runs in release, the minimum p99 under 5,000 µs;
//! - **the restart twin** (C3), **the eighteen-ground LRU** (C4), **the CI-parity pin** (C5),
//!   **the lane-diff pin** (C7), **the full-ring pin**, and the coverage test of the gate table;
//! - **the month** (C6, `sm12c_cost_lane_cohort_over_an_ic_month`, `#[ignore]`d, release only): 64
//!   TVs over the lane's six sites for 30 IC days at world rate 24, writing `cost-lane.json`.
//!
//! **The rig** is `drones.rs`'s keyed composition root (the yard, the dispatcher and the house's
//! submissions in every world) with `mill.rs`'s seed-variant `Roster` held by the rig, so a reboot
//! comes back on the same lane: every submission the composition queues is landed and measured
//! (the house's and the clerks' too — attributed house, clerk or player), every normal-lane write
//! goes through one `count_and_fold`, staff placements through `support::place_keeping`.
//!
//! **The lock**: timing that feeds a gate or a decision (C2, C4, the month) takes `exclusive()`;
//! everything else `shared()`, so the binary's wall time is not a sum and no timing is taken beside
//! a sibling. **The rate**: the sky at ephemeris rate 0 in every arm (doc 20a D15), the world at
//! rate 24 after the copied `stocked()` (which forces 1), asserted at both ends of every measured
//! span and after every reboot. **The run shape**: the month runs by hand through
//! `scripts/cost-lane.sh`, after `scripts/harness-weekly.sh` and never beside it; every other test
//! here runs in every build. Every figure prints as a `cost_lane:` line (`--nocapture`).

mod support;

#[path = "support/geology_keyed.rs"]
mod geology_keyed;

#[path = "support/cohort.rs"]
mod cohort;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Instant;

use mush_core::{
    AccountId, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId, CommitToken, GOD,
    IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext, OfferSubmission,
    RequestUid, ScopedIdempotencyKey, World,
};
use mush_economy::{EconomyConfig, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, TicketId, room_key};
use tacsim::port_tick::PortFireStats;
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::{GeologyBoot, GeologyState, RigKey, RigRecord, SurfaceFireStats, Warmed};
use tacsim::{TacsimConfig, TacsimStore};

use cohort::{Lane, WRITE_GATES};
use geology_keyed::{KEY_1, KEY_2, secret};
use support::claims::{MapOracle, install_oracle};
use support::ground::{UNSIGNED, sign};

/// The ramp's cell (every lane site's pad).
const RAMP_CELL: (i64, i64) = (128, 124);
/// Occator Rim's port entry.
const OCCATOR: &str = "occator_port";
/// IC seconds in an IC day.
const DAY: i64 = 86_400;
/// The world rate every measured span runs at (doc 20a §1.8: a span past six real days).
const RATE: u32 = 24;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The lock (doc 20a §5.3.2)
// ---------------------------------------------------------------------------

/// **The binary's lock**: timing that feeds a gate or a decision runs alone ([`exclusive`]);
/// everything else shares ([`shared`]). Poison-tolerant: a failed sibling never wedges the rest.
static LANE_LOCK: RwLock<()> = RwLock::new(());

/// The lock alone (C2, C4, the month).
fn exclusive() -> RwLockWriteGuard<'static, ()> {
    LANE_LOCK
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The lock shared (the census, C3, C5, C7, the ring pin, the coverage test).
fn shared() -> RwLockReadGuard<'static, ()> {
    LANE_LOCK
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether the timing absolutes assert (`TACSIM_ASSERT=1`, the harness's rule).
fn asserting() -> bool {
    std::env::var("TACSIM_ASSERT").is_ok_and(|v| v.trim() == "1")
}

/// Whole microseconds since `started`.
fn micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

// ---------------------------------------------------------------------------
// The roster (`mill.rs`'s, held by the rig)
// ---------------------------------------------------------------------------

/// **How a test roster is composed** (`mill.rs`'s `Roster`): the shipped game content, or a
/// seed lane in its place (booted through `support::boot_packs_with_lane`, which runs the
/// historical relay seed beside it: the full star requires the shipped lane's exact digest).
/// The sky at ephemeris rate 0 in every arm (doc 20a D15). The rig holds it, so a reboot comes
/// back on the same lane.
#[derive(Clone, Default)]
struct Roster {
    seed_lane: Option<String>,
}

impl Roster {
    fn seeded(lane: String) -> Roster {
        Roster {
            seed_lane: Some(lane),
        }
    }

    /// The loaded lane's digest this roster boots: the shipped `LANE_JSON`'s or the seed's.
    fn lane_digest(&self) -> String {
        let bytes = self.seed_lane.as_deref().unwrap_or(LANE_JSON);
        mush_content::load_pack(bytes)
            .expect("the lane verifies")
            .manifest()
            .digest()
            .to_hex()
    }

    fn modules(&self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let game = tacsim::GameEconomyConfig::default();
        let mut cfg = TacsimConfig {
            enabled: true,
            ..TacsimConfig::default()
        };
        cfg.time.ephemeris.rate = 0;
        let game_content = self.seed_lane.is_none();
        if let Some(lane) = &self.seed_lane {
            cfg.seeds.push(PackSource {
                name: "ops/lane.json".to_owned(),
                bytes: lane.clone(),
            });
        }
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.push(comp::tacsim_test_world_module_wired(
            cfg,
            game_content,
            inventory,
        ));
        match &self.seed_lane {
            None => mods
                .push(comp::content_module_with(true, true).expect("the content module composes")),
            Some(lane) => {
                let packs = support::boot_packs_with_lane(
                    mush_content::load_pack(lane).expect("the seed lane verifies"),
                );
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

// ---------------------------------------------------------------------------
// The tally: every landing by kind, attributed
// ---------------------------------------------------------------------------

/// Who a landing is attributed to (D6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Who {
    /// A yard dispatcher (`support::drones::is_house`).
    House,
    /// A lane port map's stock clerk or site clerk.
    Clerk,
    /// Everything else.
    Player,
}

/// One kind's figures.
#[derive(Clone, Debug, Default)]
struct KindRow {
    /// The action ids landed under the kind.
    actions: BTreeSet<String>,
    landed: u64,
    house: u64,
    clerk: u64,
    player: u64,
    refused: u64,
    /// The most writes one act of the kind made.
    max: usize,
    /// The constructed landings' most writes (the census's own script).
    constructed_max: Option<usize>,
    /// Variant → its most writes (constructed landings only).
    variants: BTreeMap<String, usize>,
    /// Each landing's resolve-to-install wall time, µs.
    land_us: Vec<u64>,
}

/// **Every landing of the run, by kind** (the census's and the month's recorder).
#[derive(Clone, Debug, Default)]
struct Tally {
    kinds: BTreeMap<String, KindRow>,
    /// Submissions the drain dropped (owners outside the mining lanes), by owner.
    dropped: BTreeMap<String, u64>,
    /// Refusals of landings, `"{kind}: {error}"`, the first few of each kind.
    refusals: Vec<String>,
    /// Landings refused by the queued unload's `unload_markers` (its offer while the hopper's
    /// ring is full).
    unload_markers: u64,
    /// Unload finishes that landed `Refused { MarkersFull }` (the rig record's `last`).
    markers_full: u64,
    /// The `by` of every `start.plan_end` landed (the brief's L4.1: "`start.plan_end` keeps its
    /// `by` in the report").
    plan_end_by: BTreeMap<String, u64>,
    /// **Doc 23 cut S.1: the black box pair** — the `log:hull.<n>` chunk writes landed, by kind
    /// (a launch's `Done` and an implied launch's open the epoch; a recover's `Done` closes it).
    black_box: BTreeMap<String, u64>,
}

impl Tally {
    fn row(&mut self, kind: &str) -> &mut KindRow {
        self.kinds.entry(kind.to_owned()).or_default()
    }

    /// Record one landing.
    fn landed(
        &mut self,
        (kind, action): (&str, &str),
        who: Who,
        writes: usize,
        us: u64,
        constructed: Option<&str>,
    ) {
        let row = self.row(kind);
        row.actions.insert(action.to_owned());
        row.landed = row.landed.saturating_add(1);
        match who {
            Who::House => row.house = row.house.saturating_add(1),
            Who::Clerk => row.clerk = row.clerk.saturating_add(1),
            Who::Player => row.player = row.player.saturating_add(1),
        }
        row.max = row.max.max(writes);
        row.land_us.push(us);
        if let Some(variant) = constructed {
            row.constructed_max = Some(row.constructed_max.unwrap_or(0).max(writes));
            let seen = row.variants.entry(variant.to_owned()).or_insert(0);
            *seen = (*seen).max(writes);
        }
    }

    fn refused(&mut self, kind: &str, action: &str, error: &str) {
        let row = self.row(kind);
        row.actions.insert(action.to_owned());
        row.refused = row.refused.saturating_add(1);
        if row.refused <= 3 {
            self.refusals.push(format!("{kind}: {error}"));
        }
        if error.contains("unload_markers") || error.contains("UnloadMarkers") {
            self.unload_markers = self.unload_markers.saturating_add(1);
        }
    }

    /// Every landing attributed to a player so far (C2's and C3's activity proof).
    fn player_landings(&self) -> u64 {
        self.kinds.values().map(|row| row.player).sum()
    }

    /// Every `(kind, action)` landed or refused.
    fn pairs(&self) -> Vec<(String, String)> {
        self.kinds
            .iter()
            .flat_map(|(kind, row)| {
                row.actions
                    .iter()
                    .map(move |action| (kind.clone(), action.clone()))
            })
            .collect()
    }
}

/// One module op's key, for attribution.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OpKey {
    ns: SmolStr,
    key: SmolStr,
}

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

/// A booted world, its roster, its oracle, the cumulative store, the typed post-images, the
/// harness's idempotency ledger, the pilots it spawned, the clerks, and the tally.
struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    roster: Roster,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed: BTreeMap<(SmolStr, SmolStr), Value>,
    ledger: BTreeSet<String>,
    /// Every pilot spawned (reconnected at a reboot).
    pilots: Vec<ObjId>,
    /// The census's own pilots: their landings are the test's constructed kinds.
    own: BTreeSet<ObjId>,
    /// Kinds whose clerk or house landings count as constructed while set (a landing the script
    /// set up and waits for).
    expect: BTreeSet<String>,
    /// The census variant the script is constructing.
    variant: String,
    /// The lane port maps' stock and site clerks.
    clerks: BTreeSet<ObjId>,
    tally: Tally,
    /// Module ops folded per fire (`second`).
    ops_per_fire: Vec<u64>,
    /// The pages earlier boots of this rig raised (each boot's port-tick counters are RAM, so a
    /// reboot folds them in here first; [`pages`] adds the current boot's).
    paged: BTreeMap<&'static str, u64>,
}

fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

fn with_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x53; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// **Compose `roster` over `world`, hand the key in, then run the boot pass over `store`**
/// (`drones.rs`'s `boot_over`, the roster passed in: S3).
fn boot_over(
    world: World,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    roster: &Roster,
) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, roster.modules())
        .expect("the roster composes");
    assert!(tacsim::surface::install_geology(
        &mut engine.world,
        GeologyBoot::Key(secret(KEY_1))
    ));
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(store, &mut out, &mut fx)
        .expect("the world boots");
    let tw = TestWorld::from_engine(engine);
    no_refused_writes(&tw);
    tw
}

/// The latched packs: id → (version, digest) (`tests/lane.rs`' `latches`).
fn latches(tw: &TestWorld) -> BTreeMap<String, (u32, String)> {
    tw.world()
        .module_state::<mush_content::ContentState>()
        .expect("content")
        .latches
        .all()
        .map(|l| (l.pack().to_string(), (l.version(), l.digest().to_hex())))
        .collect()
}

/// **The loaded lane is the roster's** (S3): `LANE_PACK`'s latch at `(LANE_VERSION, the roster's
/// digest)`, and `LANE_VERSION` 15.
fn assert_lane(rig: &Rig) {
    assert_eq!(LANE_VERSION, 18, "the cost lane moves no lane version");
    let latched = latches(&rig.tw).get(LANE_PACK).cloned();
    assert_eq!(
        latched,
        Some((LANE_VERSION, rig.roster.lane_digest())),
        "the loaded lane is the roster's"
    );
}

/// The lane port maps' stock and site clerks.
fn lane_clerks(tw: &TestWorld) -> BTreeSet<ObjId> {
    let Some(store) = tw.world().module_state::<TacsimStore>() else {
        return BTreeSet::new();
    };
    store
        .port_maps()
        .filter(|map| map.pack.as_str() == LANE_PACK)
        .flat_map(|map| {
            map.clerk
                .as_ref()
                .map(|clerk| ObjId(clerk.thing))
                .into_iter()
                .chain(map.site.as_ref().and_then(|site| site.clerk).map(ObjId))
        })
        .collect()
}

/// **Boot `roster` keyed, settle and stock every port** (`drones.rs`'s `stocked`, which ends at
/// world rate 1), then **world rate 24** (the brief's L3.1).
fn stocked(roster: Roster) -> Rig {
    stocked_expecting(roster, &[], "background")
}

/// [`stocked`] with `kinds` the script's from the first fire (the clerks' opening acts at a fresh
/// boot are a census's variant), under `variant`.
fn stocked_expecting(roster: Roster, kinds: &[&str], variant: &str) -> Rig {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), &roster);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    let clerks = lane_clerks(&tw);
    let mut rig = Rig {
        tw,
        oracle,
        roster,
        store: HashMap::new(),
        typed: BTreeMap::new(),
        ledger: BTreeSet::new(),
        pilots: Vec::new(),
        own: BTreeSet::new(),
        expect: kinds.iter().map(|kind| (*kind).to_owned()).collect(),
        variant: variant.to_owned(),
        clerks,
        tally: Tally::default(),
        ops_per_fire: Vec::new(),
        paged: BTreeMap::new(),
    };
    assert_lane(&rig);
    sweep(&mut rig, 120);
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    set_rate(&mut rig.tw, RATE);
    assert_eq!(world_rate(&rig.tw), RATE);
    rig
}

/// **Fold every write since the first boot and reboot keyed into the rig's own roster** (S3):
/// every connection drops (a real restart has no sessions) and every pilot the rig spawned comes
/// back; the lane latch and the world rate asserted before anything else runs.
fn reboot(rig: Rig) -> Rig {
    let Rig {
        mut tw,
        oracle,
        roster,
        mut store,
        typed,
        ledger,
        pilots,
        own,
        expect,
        variant,
        clerks: _,
        tally,
        ops_per_fire,
        mut paged,
    } = rig;
    // This boot's pages, before the reboot clears the port tick's RAM.
    for (what, n) in boot_pages(&tw) {
        *paged.entry(what).or_insert(0) += n;
    }
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
    let mut tw = boot_over(world, store.clone(), &roster);
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
    for who in &pilots {
        let _ = tw.connect(*who);
    }
    let clerks = lane_clerks(&tw);
    let rig = Rig {
        tw,
        oracle,
        roster,
        store,
        typed,
        ledger,
        pilots,
        own,
        expect,
        variant,
        clerks,
        tally,
        ops_per_fire,
        paged,
    };
    assert_lane(&rig);
    assert_eq!(world_rate(&rig.tw), RATE, "a reboot keeps the world rate");
    rig
}

/// **The pages this boot's port tick raised, by kind, the marker headroom's excepted** (the
/// brief's L5 "no page beyond the census's own", L7 "no page", L11.8 "no page beyond the marker
/// headroom's", whose firing is a figure): the drift, sky, unfunded-house, blocked-recall and
/// unreadable-voyage pages (`tacsim::port_tick`'s five readers). RAM counters of one boot.
fn boot_pages(tw: &TestWorld) -> BTreeMap<&'static str, u64> {
    let w = tw.world();
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    BTreeMap::from([
        ("drift", count(tacsim::port_tick::drift_pages(w).len())),
        ("sky", u64::from(tacsim::port_tick::sky_pages(w))),
        ("funds", u64::from(tacsim::port_tick::funds_pages(w))),
        ("recall", count(tacsim::port_tick::recall_pages(w).len())),
        ("voyage", u64::from(tacsim::port_tick::voyage_pages(w))),
    ])
}

/// **Every page the rig's world raised, across its reboots** (the earlier boots' counts folded by
/// [`reboot`] plus this boot's): only the non-zero kinds.
fn pages(rig: &Rig) -> BTreeMap<&'static str, u64> {
    let mut all = rig.paged.clone();
    for (what, n) in boot_pages(&rig.tw) {
        *all.entry(what).or_insert(0) += n;
    }
    all.retain(|_, n| *n > 0);
    all
}

/// **No page** (the marker headroom's excepted: a figure where it fires).
fn assert_no_page(rig: &Rig, what: &str) {
    let paged = pages(rig);
    assert!(paged.is_empty(), "{what}: the world paged {paged:?}");
}

/// **The one ops helper** (the brief's L3.4): every module op queued since the last call folded
/// into the rig's store, its keys returned — the only `take_module_ops` in this file but the
/// reboot's own fold (which, after a `count_and_fold`, folds nothing).
fn count_and_fold(rig: &mut Rig) -> Vec<OpKey> {
    let ops = rig.tw.take_module_ops();
    let keys = ops
        .iter()
        .map(|op| OpKey {
            ns: op.ns.clone(),
            key: match &op.op {
                ModuleWrite::JsonSet { key, .. }
                | ModuleWrite::Del { key }
                | ModuleWrite::StreamAppend { key, .. } => key.clone(),
            },
        })
        .collect();
    fold_module_ops_into(&mut rig.store, ops);
    keys
}

/// Everything due `secs` on (setup only).
fn sweep(rig: &mut Rig, secs: i64) {
    let t = rig.tw.world().now().saturating_add(secs);
    let _ = step_to(rig, t);
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
    let mut bytes = [0x6c_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Keep a prepared bundle's typed post-images for a reboot.
fn keep(rig: &mut Rig, ops: &[mush_core::ModuleOp]) {
    for op in ops {
        match &op.op {
            ModuleWrite::JsonSet { key, value } => {
                rig.typed
                    .insert((op.ns.clone(), key.clone()), value.clone());
            }
            ModuleWrite::Del { key } => {
                rig.typed.remove(&(op.ns.clone(), key.clone()));
            }
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}

/// What a landing answers for a submission already in the harness's ledger (the HIT).
const HIT: &str = "HIT: answered from the idempotency record";

/// A submission's ledger identity (`ActionCoordinator::command_ingress`'s fields).
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

/// The landing's first half: the offer projected and resolved under a fresh request token.
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

/// Who `actor` is (D6).
fn who_of(rig: &Rig, actor: ObjId) -> Who {
    if tacsim::port::yard::dispatchers(rig.tw.world()).contains(&actor) {
        Who::House
    } else if rig.clerks.contains(&actor) {
        Who::Clerk
    } else {
        Who::Player
    }
}

/// A finish's rig (`tacsim_surface_done`'s `tv` parameter).
fn finish_tv(submit: &AuthoritativeSubmit) -> Option<ObjId> {
    if submit.action.key().id.as_str() != tacsim::surface::SURFACE_DONE_ACTION {
        return None;
    }
    submit.parameters["tv"]
        .as_str()
        .and_then(|tv| tv.parse::<u32>().ok())
        .map(ObjId)
}

/// The act kind in flight on a finish's rig (its `ActKind::word`), read before the landing.
fn done_kind_of(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Option<&'static str> {
    let tv = finish_tv(submit)?;
    record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.kind.word())
}

/// What one landing did.
struct Landing {
    kind: String,
    writes: usize,
    events: Vec<(Audience, OutputEvent)>,
}

/// **The counting landing** (the brief's L3.5): the ledger's HIT, the resolve, the prepare with
/// `ops.len()` counted (SM.8's `land_counting` rule), the world task's install with the
/// observers' output; recorded under the submission's kind with its resolve-to-install wall time
/// and its attribution. A refused landing is recorded as refused under its kind, never as writes.
fn land_measured(
    rig: &mut Rig,
    submit: &AuthoritativeSubmit,
    label: Option<&'static str>,
) -> Result<Landing, String> {
    let identity = ledger_identity(submit);
    if rig.ledger.contains(&identity) {
        return Err(HIT.to_owned());
    }
    let action = submit.action.key().id.as_str().to_owned();
    let done_kind = done_kind_of(rig, submit);
    let kind = cohort::kind_of(&action, &submit.parameters, done_kind, label);
    let who = who_of(rig, submit.actor);
    let started = Instant::now();
    // Doc 23 cut S.1: the world task's composed path, so a finish's black box line rides its
    // bundle and is counted with it (the bundle's replay writes: the typed post-images, one a
    // staged op as before, and the `log:hull.<n>` chunk).
    let prepared = resolve(rig, submit).and_then(|resolved| {
        rig.tw
            .engine_mut()
            .prepare_registered_composed_resolved_authoritative_action(
                &resolved.action,
                &resolved.resolved,
                CommitToken::from_bytes(resolved.token),
            )
            .map(|(prepared, _)| prepared)
            .map_err(|error| format!("{error:?}"))
    });
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            rig.tally.refused(&kind, &action, &error);
            return Err(error);
        }
    };
    let (replay, install) = prepared.into_parts();
    let writes = replay.len();
    let (ops, ledger) = support::composed::split(&replay);
    let boxes = replay
        .iter()
        .filter(|write| support::composed::is_black_box(write))
        .count();
    assert!(boxes <= 1, "{kind}: one black box line a landing at most");
    if boxes > 0 {
        *rig.tally.black_box.entry(kind.clone()).or_insert(0) += 1;
    }
    // A count above its ceiling is a missing census case (the brief's L5): name the state so the
    // census can construct it.
    if cohort::gate(&kind).is_some_and(|gate| writes > gate.ceiling) {
        println!(
            "cost_lane: OVER {kind} {writes} by #{} {}: {:?}",
            submit.actor.0,
            submit.parameters,
            ops.iter()
                .chain(ledger.iter())
                .map(|op| match &op.op {
                    ModuleWrite::JsonSet { key, .. } => format!("set {}:{key}", op.ns),
                    ModuleWrite::Del { key } => format!("del {}:{key}", op.ns),
                    ModuleWrite::StreamAppend { key, .. } => format!("append {}:{key}", op.ns),
                })
                .collect::<Vec<_>>()
        );
    }
    let events = rig.tw.install_composed_action_announcing(install);
    let us = micros(started);
    keep(rig, &ops);
    fold_module_ops_into(&mut rig.store, ledger);
    rig.ledger.insert(identity);
    // The outcomes the rings' figures read (the brief's L11.7): an unload finish that landed
    // refused `MarkersFull` (its rig record's `last`), and the `by` a plan's end carried.
    if kind == "done.unload"
        && let Some(tv) = finish_tv(submit)
        && record_of(&mut rig.tw, tv)
            .and_then(|record| record.last)
            .is_some_and(|last| {
                last.result
                    == (tacsim::surface::ActResult::Refused {
                        code: tacsim::surface::RefusedCode::MarkersFull,
                    })
            })
    {
        rig.tally.markers_full = rig.tally.markers_full.saturating_add(1);
    }
    if kind == "start.plan_end" {
        let by = submit.parameters["by"]
            .as_str()
            .unwrap_or("none")
            .to_owned();
        *rig.tally.plan_end_by.entry(by).or_insert(0) += 1;
    }
    let constructed = (rig.own.contains(&submit.actor)
        || label.is_some()
        || (who != Who::Player && rig.expect.contains(&kind)))
    .then(|| rig.variant.clone());
    rig.tally
        .landed((&kind, &action), who, writes, us, constructed.as_deref());
    Ok(Landing {
        kind,
        writes,
        events,
    })
}

/// Land every submission (each a landing or the ledger's HIT); refusals recorded under their kind.
/// The events every landing announced.
fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) -> Vec<(Audience, OutputEvent)> {
    let mut events = Vec::new();
    for submit in queued {
        if let Ok(landing) = land_measured(rig, submit, None) {
            events.extend(landing.events);
        }
    }
    events
}

/// **Every queued submission of the mining lanes** (`drones.rs`'s owner filter: tacsim,
/// inventory, market, industry) — every other owner's dropped and counted as
/// `other.dropped.<owner>` (D6; their writes are not measured). GOD never submits from a tick.
fn drain(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str().to_owned();
        if owner == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from a tick");
        }
        if owner == tacsim::NS || owner == "inventory" || owner == "market" || owner == "industry" {
            kept.push(*submit);
        } else {
            *rig.tally
                .dropped
                .entry(format!("other.dropped.{owner}"))
                .or_insert(0) += 1;
        }
    }
    kept
}

/// **What one step of the clock did** (the brief's L3.3).
struct Fired {
    /// The surface fire's stats, when the surface tick fired.
    surface: Option<SurfaceFireStats>,
    /// The port fire's stats, when the port tick fired.
    port: Option<PortFireStats>,
    /// The whole `fire_due`'s wall time (every tick of the composition that fired), µs.
    fire_us: u64,
    /// The submissions the fire queued (landed after the clock stopped).
    queued: Vec<AuthoritativeSubmit>,
    /// The fire's own module ops (before any landing), for tick-borne normal-lane writes.
    ops: Vec<OpKey>,
    /// The landings' and observers' module ops.
    landing_ops: usize,
    /// Everything the fire and its landings said.
    events: Vec<(Audience, OutputEvent)>,
    /// The kinds landed after the fire, with their actors.
    landed: Vec<(String, ObjId)>,
    /// The Ceres clerk's act this fire, if a port fire read one.
    clerk: Option<ClerkAct>,
}

/// **One step of the clock to real second `t`**: `fire_due` timed as a whole, the fire's own
/// module ops folded, then every queued submission landed (outside the fire's clock), the
/// landings' ops folded, and the tick-borne normal-lane writes attributed.
fn step_to(rig: &mut Rig, t: i64) -> Fired {
    let surface_before = tacsim::surface::surface_fires(rig.tw.world());
    let port_before = tacsim::port_tick::fires(rig.tw.world());
    let before = tick_read(&rig.tw);
    rig.tw.set_now(t);
    let started = Instant::now();
    let mut events = rig.tw.fire_due(t.saturating_mul(1_000));
    let fire_us = micros(started);
    let w = rig.tw.world();
    let surface = (tacsim::surface::surface_fires(w) != surface_before)
        .then(|| tacsim::surface::last_surface_fire(w))
        .flatten();
    let port = (tacsim::port_tick::fires(w) != port_before)
        .then(|| tacsim::port_tick::last_fire(w))
        .flatten();
    let ops = count_and_fold(rig);
    let queued = drain(rig);
    let mut landed = Vec::new();
    for submit in &queued {
        if let Ok(landing) = land_measured(rig, submit, None) {
            landed.push((landing.kind, submit.actor));
            events.extend(landing.events);
        }
    }
    let landing_ops = count_and_fold(rig).len();
    let after = tick_read(&rig.tw);
    let clerk = tick_writes(rig, (&before, &after), &ops, &landed);
    Fired {
        surface,
        port,
        fire_us,
        queued,
        ops,
        landing_ops,
        events,
        landed,
        clerk,
    }
}

/// One six-second fire, everything landed (setup only; never measured); what it said.
fn fire(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
    let t = rig.tw.world().now().saturating_add(6);
    step_to(rig, t).events
}

/// **One measured second** (the brief's L3.3; production's cadence): [`step_to`] `now + 1`.
fn second(rig: &mut Rig) -> Fired {
    let t = rig.tw.world().now().saturating_add(1);
    let fired = step_to(rig, t);
    rig.ops_per_fire
        .push(u64::try_from(fired.ops.len().saturating_add(fired.landing_ops)).unwrap_or(u64::MAX));
    fired
}

/// Fire and land the port tick until every port is stocked and listed (`drones.rs`'s).
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
            })
            .collect();
        if gated.len() > 1 && gated.iter().all(|line| line.ends_with("[ready]")) {
            return;
        }
        let _ = fire(rig);
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

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
}

fn inventory(tw: &TestWorld) -> mush_inventory::InventoryFamilies {
    tw.world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone()
}

/// Stage `body` in one inventory transaction and install it (staff's hand); the typed
/// post-images kept.
fn staff_inventory(
    rig: &mut Rig,
    body: impl FnOnce(
        &mush_inventory::InventoryFamilies,
        &mut mush_core::ActionTxn<'_>,
    ) -> Result<(), mush_inventory::InventoryError>,
) {
    let families = inventory(&rig.tw);
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("an inventory transaction");
        body(&families, &mut txn).expect("staff's inventory write");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, &ops);
}

/// Seed `qty` of `good` into `vault`'s pool through `lot` (the inventory's own path).
fn seed_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
    staff_inventory(rig, |families, txn| {
        families
            .mint_origin(
                txn,
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
    });
    staff_inventory(rig, |families, txn| {
        families.dissolve(txn, endpoint, lot).map(|_| ())
    });
}

/// A pool's quantity.
fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = inventory(tw);
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    families
        .pool(&mut txn, EndpointRef(vault), &cid(good))
        .expect("the pool reads")
        .map_or(0, |pool| pool.quantity)
}

/// A vault's marker ring, as the inventory keeps it.
fn marker_ring(tw: &mut TestWorld, vault: VaultId) -> Option<mush_inventory::record::MarkerRing> {
    let families = inventory(tw);
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("a transaction");
    txn.get(&families.marker_ring, &EndpointRef(vault))
        .expect("the family reads")
        .cloned()
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

/// A rig's typed record.
fn record_of(tw: &mut TestWorld, tv: ObjId) -> Option<RigRecord> {
    let h = handles(tw);
    typed(tw, &h.families.excursions, &RigKey(tv))
}

/// A principal's chart row for a site.
fn chart_of(tw: &mut TestWorld, who: ObjId, entry: &str) -> Option<tacsim::surface::ChartRecord> {
    let h = handles(tw);
    typed(
        tw,
        &h.families.charts,
        &tacsim::surface::ChartKey {
            principal: who,
            site: cid(entry),
        },
    )
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

/// Whether `entry`'s ground is cached and its home field built.
fn warm_at(tw: &TestWorld, entry: &str) -> bool {
    let (spec, location) = spec_of(tw, entry);
    let cached = tacsim::surface::site_ground_cached(tw.world(), &site_frame(&location), &spec)
        .ok()
        .flatten()
        .is_some();
    let homed = tw
        .world()
        .module_state::<GeologyState>()
        .is_some_and(|state| {
            state
                .homes()
                .iter()
                .any(|(site, ramp, _)| site.as_str() == entry && *ramp == RAMP_CELL)
        });
    cached && homed
}

/// Fire (six-second setup fires) until `entry`'s ground is cached and its home built.
fn warm(rig: &mut Rig, entry: &str) {
    for _ in 0..12 {
        if warm_at(&rig.tw, entry) {
            return;
        }
        let _ = fire(rig);
    }
    panic!("{entry}'s ground was not warmed");
}

// ---------------------------------------------------------------------------
// The player's side (`drones.rs`'s and `claims.rs`'s helpers, copied; the licence and the lease
// counted by their command's own module ops)
// ---------------------------------------------------------------------------

/// Jump to the first real second whose IC instant is at or past `target`; one fire there, every
/// submission landed; what it said.
fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = ((ahead + rate - 1) / rate).max(1);
    let t = rig.tw.world().now().saturating_add(real);
    step_to(rig, t).events
}

/// A command and the submissions it queued (not landed).
fn command(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(rig);
    (out, queued)
}

/// **`line` as `who`, exactly one submission queued by `who`**, landed under `label`; the reply
/// and the landing's kind and writes.
fn act_go_as(
    rig: &mut Rig,
    who: ObjId,
    line: &str,
    label: Option<&'static str>,
) -> (String, String, usize) {
    let (out, queued) = command(rig, who, line);
    let mine: Vec<&AuthoritativeSubmit> = queued.iter().filter(|s| s.actor == who).collect();
    assert_eq!(mine.len(), 1, "{line}: one submission: {out}");
    let mut landed = None;
    for submit in &queued {
        let label = if submit.actor == who { label } else { None };
        match land_measured(rig, submit, label) {
            Ok(landing) if submit.actor == who => landed = Some((landing.kind, landing.writes)),
            Ok(_) => {}
            Err(error) if submit.actor == who => panic!("{line}: did not land: {error}"),
            Err(_) => {}
        }
    }
    let _ = count_and_fold(rig);
    let (kind, writes) = landed.expect("the submission landed");
    (out, kind, writes)
}

/// [`act_go_as`] unlabelled; the reply.
fn act_go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    act_go_as(rig, who, line, None).0
}

/// **A synchronous normal-lane command counted** (the brief's L4.1; D7): the queued ops folded
/// first, `line` run, its module ops folded and counted under `kind` (attributed to `who`); no
/// authoritative submission may be queued by it. The reply and the writes.
fn normal_cmd(rig: &mut Rig, who: ObjId, line: &str, kind: &str) -> (String, usize) {
    let _ = count_and_fold(rig);
    let started = Instant::now();
    let out = run(&mut rig.tw, who, line);
    let us = micros(started);
    let ops = count_and_fold(rig);
    let queued = drain(rig);
    assert!(
        queued.is_empty(),
        "{line}: a normal-lane command submits nothing: {queued:?}"
    );
    let variant = rig.variant.clone();
    let attributed = who_of(rig, who);
    rig.tally.landed(
        (kind, &format!("normal:{kind}")),
        attributed,
        ops.len(),
        us,
        Some(&variant),
    );
    (out, ops.len())
}

/// A pilot on a gateway account of their own, standing at the Registry desk; one of the rig's
/// pilots (reconnected at a reboot) and the census's own.
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
    rig.pilots.push(pilot);
    rig.own.insert(pilot);
    pilot
}

/// **A chartered, licensed pilot whose hull the yard has made a Dray-P** (`+coop/enrol`,
/// `+coop/refit prospector`, `+licence/buy go` — the licence counted as `licence.buy`).
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
    let (out, _) = normal_cmd(rig, pilot, "+licence/buy go", "licence.buy");
    assert!(
        out.contains("licenses you to prospect at its sites until"),
        "the pilot is licensed: {out}"
    );
    (pilot, row)
}

/// The hull's hold vault.
fn hold_vault(tw: &TestWorld, row: &HullRow) -> VaultId {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
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

/// Lease one rig with `+tv/lease go` at Occator's counter, counted as `lease.take`.
fn lease_one(rig: &mut Rig, pilot: ObjId) -> usize {
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, counter);
    let (out, writes) = normal_cmd(rig, pilot, "+tv/lease go", "lease.take");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );
    writes
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

/// File a passage with `line`, land its burn, launch it on the next fire; its arrival IC.
fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> i64 {
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
    let _ = land_all(rig, &queued);
    let h = handles(&rig.tw);
    let ticket =
        typed(&mut rig.tw, &h.families.tickets, &TicketId { hull, seq }).expect("the ticket");
    let _ = fire(rig);
    ticket.plan.depart_ic + ticket.plan.total_s
}

/// **Fire (and land everything) until `entry`'s Site line reads Ready** (the clerk's opening
/// acts landed and measured as `site.open` and `site.worked`), at most 200 fires.
fn open_site(rig: &mut Rig, entry: &str) {
    for _ in 0..200 {
        if support::sites::site_open(&rig.tw, entry) {
            return;
        }
        let _ = fire(rig);
    }
    panic!(
        "{entry} did not open within 200 fires: {:?}",
        support::sites::site_line(&rig.tw, entry)
    );
}

/// **Finish `tv`'s act**: to its due instant (the fire there queues the finish, landed), then one
/// more six-second fire for the tell; what `who` heard over both.
fn finish(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight");
    let events = to_ic(rig, due);
    let mut heard = said(&rig.tw, &events, who);
    let events = fire(rig);
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, who));
    heard
}

/// **Staff place `tv` `Direct` on `entry`'s domain at site cell `cell`** (`support::place_keeping`,
/// SM.11's rule): the world's own ops folded first, then the placement's.
fn to_cell_on(rig: &mut Rig, entry: &str, tv: ObjId, cell: (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, entry).domain.expect("a domain"));
    let staff = support::place_keeping(&mut rig.tw, domain, tv, cell);
    let _ = count_and_fold(rig);
    fold_module_ops_into(&mut rig.store, staff);
}

/// The soft stands of block `letter` of `entry` (`claims.rs`'s), nearest the block's middle
/// first, each with its pocket (from the keyed truth a test may read).
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

/// **The first pocket of `entry` with ore of exactly two families**, standing off rock, from the
/// keyed truth a test may read (`mining.rs`'s `picks` and `pick(.., families() == 2)`), none of
/// `skip`: its address and its stand cell.
fn two_family_pocket(tw: &mut TestWorld, entry: &str, skip: &[String]) -> (String, (i64, i64)) {
    let ground = ground_of(tw, entry);
    let surface = ground
        .composite
        .surface(&mush_atlas::SurfaceRef::Ground)
        .expect("a ground surface");
    for py in 0..64_i64 {
        for px in 0..64_i64 {
            let Some(row) = ground.geology.pocket(px, py) else {
                continue;
            };
            let families = row
                .classes
                .iter()
                .filter(|class| **class != tacsim::surface::geology::Word::Barren)
                .count();
            let stand = (px * 4 + 2, py * 4 + 2);
            let soft = tacsim::surface::chart::footing_at(surface, stand)
                .is_ok_and(|footing| footing != tacsim::surface::reading::Footing::Rock);
            let Some(addr) = tacsim::surface::address::pocket_address(px, py) else {
                continue;
            };
            if families == 2 && soft && !skip.contains(&addr) {
                println!(
                    "cost_lane: C1 two-family pocket {addr} classes {:?} tonnes {:?}",
                    row.classes, row.tonnes
                );
                return (addr, stand);
            }
        }
    }
    panic!("{entry} has no two-family pocket on soft ground");
}

/// Write `record` into the chart family through a typed transaction (staff's hand).
fn put_chart(rig: &mut Rig, record: tacsim::surface::ChartRecord) {
    let family = handles(&rig.tw).families.charts;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("a chart transaction");
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
    keep(rig, &ops);
}

/// A rig's hopper vault.
fn hopper(tw: &TestWorld, tv: ObjId) -> VaultId {
    tacsim::port::vehicle::hopper_of(tw.world(), tv).expect("a hopper vault")
}

/// Seed `qty` of `good` into `tv`'s hopper (staff's hand, no marker).
fn fill(rig: &mut Rig, tv: ObjId, good: &str, qty: u64, lot: u64) {
    let vault = hopper(&rig.tw, tv);
    seed_pool(rig, vault, good, qty, LotId(lot));
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
    keep(rig, &ops);
}

/// A full battery out of the bay at the world's IC now (staff's hand, SM.10's seam).
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

// ---------------------------------------------------------------------------
// The census's close: the writes lines, the gates, the equality, the leak guard
// ---------------------------------------------------------------------------

/// **The leak guard** (the brief's L4.2): every landed kind gated or `other.*`, and no mining id
/// in `other.*`.
fn assert_kinds_classified(rig: &Rig) {
    let pairs = rig.tally.pairs();
    let offenders = cohort::unclassified(pairs.iter().map(|(k, a)| (k.as_str(), a.as_str())));
    assert!(
        offenders.is_empty(),
        "unclassified landings (map each, with a census variant): {offenders:?}"
    );
}

/// **Every landed gated kind within its ceiling** (both the script's and the background's).
fn assert_within_gates(rig: &Rig) {
    for (kind, row) in &rig.tally.kinds {
        if let Some(gate) = cohort::gate(kind) {
            assert!(
                row.max <= gate.ceiling,
                "{kind}: {} writes over its ceiling {} ({}) — a missing census case, never a \
                 raised const",
                row.max,
                gate.ceiling,
                gate.basis.word()
            );
        }
    }
}

/// Print the tally: one `writes` line per constructed kind, one `background` line per other.
fn print_tally(rig: &Rig, test: &str) {
    for (kind, row) in &rig.tally.kinds {
        let (ceiling, basis, lane) = cohort::gate(kind).map_or_else(
            || ("-".to_owned(), "ungated".to_owned(), "-"),
            |gate| {
                (
                    gate.ceiling.to_string(),
                    gate.basis.word(),
                    gate.lane.word(),
                )
            },
        );
        match row.constructed_max {
            Some(max) => println!(
                "cost_lane: writes {kind} max {max} ceiling {ceiling} basis {basis} lane {lane} \
                 variants {:?} house {} clerk {} player {} refused {} ({test})",
                row.variants, row.house, row.clerk, row.player, row.refused
            ),
            None => println!(
                "cost_lane: background {kind} {} house {} clerk {} player {} max {} refused {} \
                 ({test})",
                row.landed, row.house, row.clerk, row.player, row.max, row.refused
            ),
        }
    }
    for (owner, n) in &rig.tally.dropped {
        println!("cost_lane: {owner} {n} ({test})");
    }
    for refusal in &rig.tally.refusals {
        println!("cost_lane: refused {refusal} ({test})");
    }
    println!(
        "cost_lane: outcomes unload_markers refusals {} MarkersFull finishes {} plan_end by {:?}; \
         pages {:?} ({test})",
        rig.tally.unload_markers,
        rig.tally.markers_full,
        rig.tally.plan_end_by,
        pages(rig)
    );
}

/// **A census test's close** (the brief's L5 rules): the tally printed; every gated kind within
/// its ceiling; the constructed kinds equal to the test's `CENSUS` row exactly; each `Measured`
/// row equal to its constructed maximum in the first test that lists it; the leak guard; no
/// refused namespace write; no page (no census here pages by design, so none is whitelisted).
fn census_close(rig: &Rig, test: &str) {
    census_close_paging(rig, test, &BTreeMap::new());
}

/// [`census_close`] whose world raised exactly `own`, the census's own pages (each named with its
/// cause where the census calls this).
fn census_close_paging(rig: &Rig, test: &str, own: &BTreeMap<&'static str, u64>) {
    print_tally(rig, test);
    assert_within_gates(rig);
    let row = cohort::CENSUS
        .iter()
        .find(|(name, _)| *name == test)
        .map(|(_, kinds)| *kinds)
        .unwrap_or_else(|| panic!("{test} has no CENSUS row"));
    let want: BTreeSet<&str> = row.iter().copied().collect();
    // Gated kinds only: an `other.*` kind the script waited for (the relisting) is printed, never
    // part of the census's equality.
    let constructed: BTreeSet<&str> = rig
        .tally
        .kinds
        .iter()
        .filter(|(kind, row)| row.constructed_max.is_some() && cohort::gate(kind).is_some())
        .map(|(kind, _)| kind.as_str())
        .collect();
    let missing: Vec<&&str> = want.difference(&constructed).collect();
    let extra: Vec<&&str> = constructed.difference(&want).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{test}: the constructed kinds differ from its CENSUS row: missing {missing:?}, extra \
         {extra:?}"
    );
    for gate in WRITE_GATES {
        let pinned_here = cohort::MEASURED_AT
            .iter()
            .any(|(kind, at)| *kind == gate.kind && *at == test);
        if pinned_here {
            assert_eq!(
                rig.tally
                    .kinds
                    .get(gate.kind)
                    .and_then(|row| row.constructed_max),
                Some(gate.ceiling),
                "{}: a Measured ceiling is its exact constructed maximum",
                gate.kind
            );
        }
    }
    assert_kinds_classified(rig);
    no_refused_writes(&rig.tw);
    assert_eq!(
        &pages(rig),
        own,
        "{test}: the world paged beyond the census's own"
    );
}

/// The constructed maximum of `kind` (the census's exact pins).
fn constructed(rig: &Rig, kind: &str) -> usize {
    rig.tally
        .kinds
        .get(kind)
        .and_then(|row| row.constructed_max)
        .unwrap_or_else(|| panic!("{kind} was not constructed"))
}

/// The constructed figure of one `variant` of `kind`.
fn variant_of(rig: &Rig, kind: &str, variant: &str) -> usize {
    rig.tally
        .kinds
        .get(kind)
        .and_then(|row| row.variants.get(variant).copied())
        .unwrap_or_else(|| panic!("{kind} variant {variant} was not constructed"))
}

// ---------------------------------------------------------------------------
// The coverage test (pure)
// ---------------------------------------------------------------------------

/// **Every gated kind has a census and every mining id a kind** (doc 20a §5.3.3; the brief's
/// L4.4): the union of `CENSUS` equals `WRITE_GATES`' kinds both ways; `kind_of` maps every
/// `ACT_WIRE_WORDS` word and every mining-layer id — the claim receipts with each of their labels
/// and prefixes — to a gated kind; an unlabelled player claim reads `claim.unlabelled`, which
/// `unclassified` returns; an unmapped `industry_*` id and a mining id are never `other.*`.
#[test]
fn sm12c_every_gated_kind_has_a_census_and_every_mining_id_a_kind() {
    let _lock = shared();
    cohort::self_check();
    let gated: BTreeSet<&str> = WRITE_GATES.iter().map(|gate| gate.kind).collect();
    assert_eq!(gated.len(), WRITE_GATES.len(), "one gate a kind");
    let census: BTreeSet<&str> = cohort::CENSUS
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
        .collect();
    assert_eq!(
        census.symmetric_difference(&gated).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "the census covers exactly the gated kinds"
    );
    let check = |kind: String| {
        assert!(cohort::gate(&kind).is_some(), "{kind} is gated");
        kind
    };
    let surface = tacsim::surface::act::SURFACE_ACT_ACTION;
    for word in tacsim::surface::act::ACT_WIRE_WORDS {
        let _ = check(cohort::kind_of(
            surface,
            &json!({ "act": word }),
            None,
            None,
        ));
    }
    for word in ["drive", "mine", "recover", "sound"] {
        let kind = check(cohort::kind_of(
            surface,
            &json!({ "act": word, "by": "plan" }),
            None,
            None,
        ));
        assert_eq!(kind, format!("start.{word}.plan"));
    }
    assert_eq!(
        check(cohort::kind_of(
            surface,
            &json!({ "act": "plan_end", "by": "plan" }),
            None,
            None
        )),
        "start.plan_end"
    );
    assert_eq!(
        check(cohort::kind_of(
            surface,
            &json!({ "act": "recover", "by": "idle" }),
            None,
            None
        )),
        "start.recover.idle"
    );
    assert_eq!(
        check(cohort::kind_of(
            surface,
            &json!({ "act": "unload", "how": "queued" }),
            None,
            None
        )),
        "start.unload.queued"
    );
    let done = tacsim::surface::act::SURFACE_DONE_ACTION;
    for word in [
        "launch", "recover", "dump", "scan", "sound", "drive", "unload",
    ] {
        let _ = check(cohort::kind_of(
            done,
            &json!({ "tv": "1" }),
            Some(word),
            None,
        ));
    }
    assert_eq!(
        check(cohort::kind_of(done, &json!({ "cycle": 2 }), None, None)),
        "done.cycle"
    );
    let tenure = mush_industry::ACTION_CLAIM_TENURE;
    for label in cohort::CLAIM_LABELS {
        assert_eq!(
            check(cohort::kind_of(
                tenure,
                &json!({ "receipt": "tacsim.claim/1/2" }),
                None,
                Some(label)
            )),
            format!("claim.{label}")
        );
    }
    for (receipt, kind) in [
        ("tacsim.claim/1/release/ahuna_port/c/0", "claim.release"),
        (
            &*format!(
                "{}ahuna_port/c/0",
                tacsim::port::site::CLAIM_UNPAID_KEY_PREFIX
            ),
            "claim.unpaid",
        ),
        (
            &*format!(
                "{}ahuna_port/c/0",
                tacsim::port::site::CLAIM_LAPSE_KEY_PREFIX
            ),
            "claim.lapse",
        ),
        (
            &*format!(
                "{}occator_port/f/occator",
                tacsim::port::site::CLAIM_SEED_KEY_PREFIX
            ),
            "claim.seed",
        ),
    ] {
        assert_eq!(
            check(cohort::kind_of(
                tenure,
                &json!({ "receipt": receipt }),
                None,
                None
            )),
            kind
        );
    }
    // The unlabelled-claim guard: the pure core reads it, the leak guard names it.
    let unlabelled = cohort::kind_of(
        tenure,
        &json!({ "receipt": "tacsim.claim/1/2" }),
        None,
        None,
    );
    assert_eq!(unlabelled, "claim.unlabelled");
    assert_eq!(
        cohort::unclassified([(unlabelled.as_str(), tenure)]),
        vec![format!("claim.unlabelled ({tenure})")]
    );
    for (action, params, kind) in [
        (
            tacsim::surface::chart::CHART_FORGET_ACTION,
            json!({}),
            "forget",
        ),
        (
            tacsim::port::site::SITE_WORK_ACTION,
            json!({}),
            "site.worked",
        ),
        (
            mush_industry::provider::ACTION_DEPOSIT_OPEN,
            json!({}),
            "site.open",
        ),
        (tacsim::port::mill::MILL_ACTION, json!({}), "mill.batch"),
        (
            tacsim::port::yard::DISPATCH_ACTION,
            json!({ "act": "deliver" }),
            "dispatch.deliver",
        ),
        (
            tacsim::port::yard::DISPATCH_ACTION,
            json!({ "act": "survey" }),
            "dispatch.survey",
        ),
    ] {
        assert_eq!(check(cohort::kind_of(action, &params, None, None)), kind);
    }
    // Never `other.*` for a mining id; `other.*` for everything else.
    assert_eq!(
        cohort::kind_of("industry_extract_cycle", &json!({}), None, None),
        "industry.extract_cycle"
    );
    assert_eq!(
        cohort::kind_of(tacsim::port::stock::STOCK_ACTION, &json!({}), None, None),
        "other.tacsim_port_stock"
    );
    assert!(cohort::unclassified([("other.x", "tacsim_surface_act")]).len() == 1);
    assert!(cohort::unclassified([("other.x", "market_draw")]).is_empty());
    assert_eq!(
        cohort::unclassified([("industry.extract_cycle", "industry_extract_cycle")]).len(),
        1
    );
    // Every `Measured` gate (and `done.unload`'s constructed pin) is pinned exactly by one census
    // test whose row lists it.
    for gate in WRITE_GATES {
        let exact = gate.basis == cohort::Basis::Measured || gate.kind == "done.unload";
        let at = cohort::MEASURED_AT
            .iter()
            .find(|(kind, _)| *kind == gate.kind)
            .map(|(_, test)| *test);
        assert_eq!(exact, at.is_some(), "{}: one exact pin", gate.kind);
        if let Some(test) = at {
            assert!(
                cohort::CENSUS
                    .iter()
                    .any(|(name, kinds)| *name == test && kinds.contains(&gate.kind)),
                "{}: pinned by {test}, whose row lists it",
                gate.kind
            );
        }
    }
    for gate in WRITE_GATES {
        println!(
            "cost_lane: gate {} ceiling {} basis {} lane {}",
            gate.kind,
            gate.ceiling,
            gate.basis.word(),
            gate.lane.word()
        );
        if gate.lane == Lane::Bundle && gate.basis == cohort::Basis::DocCompletion {
            assert!(gate.kind.starts_with("done."), "{}", gate.kind);
        }
    }
}

// ---------------------------------------------------------------------------
// C5 — the CI comment's two lines are the script's (O-28 default (i))
// ---------------------------------------------------------------------------

/// The workspace root.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// **C5 — the cost lane's script runs the CI comment's two lines verbatim** (doc 20a §5.3.13;
/// O-28's default (i), provisional): `ci.yml`'s comment block "Doc 20 cut SM.12c-lane" carries
/// exactly two `# cargo test` lines equal, in order, to `scripts/cost-lane.sh`'s; it names the
/// month and the script; SM.12c-bot's block still names its cohort; both blocks lie before the
/// `tacsim-harness` job (outside its parity cut) and no job runs `--test cost_lane`; the script
/// exports `TACSIM_ASSERT=1` and `TACSIM_COST_JSON=cost-lane.json`, runs under `set -euo pipefail`
/// and says it runs after the harness, never beside it.
#[test]
fn sm12c_the_cost_lane_script_runs_the_ci_comments_lines_verbatim() {
    let _lock = shared();
    let root = workspace_root();
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    };
    let ci = read(".github/workflows/ci.yml").replace("\r\n", "\n");
    let script = read("scripts/cost-lane.sh").replace("\r\n", "\n");
    let head = "  # Doc 20 cut SM.12c-lane";
    let start = ci
        .find(head)
        .expect("ci.yml carries the cost lane's comment block");
    let block: Vec<&str> = ci[start..]
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .collect();
    let block_lines: Vec<&str> = block
        .iter()
        .filter_map(|line| line.strip_prefix("  # cargo test"))
        .collect();
    let script_lines: Vec<&str> = script
        .lines()
        .filter_map(|line| line.trim().strip_prefix("cargo test"))
        .collect();
    assert_eq!(
        block_lines.len(),
        2,
        "two by-hand lines:\n{}",
        block.join("\n")
    );
    assert_eq!(
        script_lines, block_lines,
        "scripts/cost-lane.sh and the ci.yml comment run different cargo lines"
    );
    let text = block.join("\n");
    assert!(
        text.contains("sm12c_cost_lane_cohort_over_an_ic_month"),
        "{text}"
    );
    assert!(text.contains("scripts/cost-lane.sh"), "{text}");
    assert!(
        ci.contains("four_bots_mine_and_sell_over_ssh_in_line_mode"),
        "SM.12c-bot's block still names its cohort"
    );
    let harness = ci
        .find("\n  tacsim-harness:")
        .expect("ci.yml has a tacsim-harness job");
    let bot = ci
        .find("  # Doc 20 cut SM.12c-bot")
        .expect("SM.12c-bot's block");
    assert!(
        start < harness && bot < harness,
        "both blocks lie before the harness job"
    );
    assert!(
        !ci.lines()
            .any(|line| line.trim_start().starts_with("run:") && line.contains("--test cost_lane")),
        "no job runs the cost lane (O-28 (i))"
    );
    for want in [
        "export TACSIM_ASSERT=1",
        "export TACSIM_COST_JSON=cost-lane.json",
        "set -euo pipefail",
        "run it after scripts/harness-weekly.sh, never beside it",
    ] {
        assert!(script.contains(want), "scripts/cost-lane.sh: {want}");
    }
}

// ---------------------------------------------------------------------------
// The census's own helpers
// ---------------------------------------------------------------------------

/// Ahuna Flank's port entry (the census's site; `unloading.rs`'s and `plans.rs`'s).
const AHUNA: &str = "ahuna_port";
/// Kerwan Floor's port entry: the tenure census's site, whose blocks each open all four families'
/// deposits (Ahuna Flank's open three), so its claim acts write the most.
const KERWAN: &str = "kerwan_port";

/// **The pockets a sounding named for `family`** (the pilot's own reading, from the completion
/// block's `"  {family}: {addr} {class}, …"` line), richest first, then in the listed order.
fn sounded_pockets(text: &str, family: &str) -> Vec<(String, String)> {
    let head = format!("  {family}: ");
    let mut out: Vec<(u8, usize, String, String)> = Vec::new();
    for line in text.lines() {
        let Some(list) = line.strip_prefix(&head) else {
            continue;
        };
        for (at, entry) in list.split(", ").enumerate() {
            let Some((addr, class)) = entry.split_once(' ') else {
                continue;
            };
            let rank = match class {
                "rich" => 0,
                "fair" => 1,
                "lean" => 2,
                _ => continue,
            };
            if !out.iter().any(|(_, _, seen, _)| seen == addr) {
                out.push((rank, at, addr.to_owned(), class.to_owned()));
            }
        }
    }
    out.sort();
    out.into_iter()
        .map(|(_, _, addr, class)| (addr, class))
        .collect()
}

/// A command that may queue one submission by `who`: `Some` with the reply, kind and writes when
/// it did (landed), `None` (and the reply) when it queued nothing.
fn try_act(rig: &mut Rig, who: ObjId, line: &str) -> Result<(String, String, usize), String> {
    let (out, queued) = command(rig, who, line);
    if !queued.iter().any(|s| s.actor == who) {
        let _ = land_all(rig, &queued);
        return Err(out);
    }
    let mut landed = None;
    for submit in &queued {
        match land_measured(rig, submit, None) {
            Ok(landing) if submit.actor == who => landed = Some((landing.kind, landing.writes)),
            Ok(_) | Err(_) => {}
        }
    }
    let _ = count_and_fold(rig);
    match landed {
        Some((kind, writes)) => Ok((out, kind, writes)),
        None => Err(format!("{line}: queued but did not land: {out}")),
    }
}

/// `line` as `who` under census variant `variant`, its act finished; what `who` heard.
fn act_and_finish(rig: &mut Rig, who: ObjId, tv: ObjId, line: &str, variant: &str) -> String {
    rig.variant = variant.to_owned();
    let out = act_go(rig, who, line);
    let heard = finish(rig, tv, who);
    format!("{out}\n{heard}")
}

/// **A run's cycles landed one at a time**, each under the variant `{base} k={k}`; what `who`
/// heard.
fn cycles_named(rig: &mut Rig, tv: ObjId, who: ObjId, base: &str) -> String {
    let mut heard = String::new();
    for _ in 0..12 {
        let Some(run) = record_of(&mut rig.tw, tv).and_then(|record| record.run) else {
            break;
        };
        rig.variant = format!("{base} k={}", run.k);
        let events = to_ic(rig, run.due_ic);
        heard.push_str(&said(&rig.tw, &events, who));
        heard.push('\n');
    }
    let events = fire(rig);
    heard.push_str(&said(&rig.tw, &events, who));
    heard
}

/// Drive rig `slot` to the first of `pockets` it may reach and mine it out (`+tv/<slot> mine
/// go`, every cycle under `base`); the pocket mined, `None` when none could start.
fn drive_and_mine(
    rig: &mut Rig,
    (who, slot, tv): (ObjId, u8, ObjId),
    pockets: &[String],
    base: &str,
) -> Option<String> {
    for addr in pockets {
        rig.variant = format!("{base}: drive");
        let Ok(_) = try_act(rig, who, &format!("+tv/{slot} drive {addr} go")) else {
            continue;
        };
        let _ = finish(rig, tv, who);
        rig.variant = format!("{base}: mine");
        if try_act(rig, who, &format!("+tv/{slot} mine go")).is_err() {
            continue;
        }
        let _ = cycles_named(rig, tv, who, base);
        return Some(addr.clone());
    }
    None
}

/// Whether `tv` still has work: a live plan, an act, a run or a queued unload (`plans.rs`'s).
fn busy(tw: &mut TestWorld, tv: ObjId) -> bool {
    record_of(tw, tv).is_some_and(|record| {
        record.act.is_some()
            || record.run.is_some()
            || record.queued.is_some()
            || record
                .plan
                .as_ref()
                .is_some_and(tacsim::surface::plan::FillPlan::live)
    })
}

/// The next instant `tvs` need the clock at (`plans.rs`'s `next_instant`).
fn next_instant(rig: &mut Rig, tvs: &[ObjId]) -> i64 {
    let now = ic_now(&rig.tw);
    let mut next: Option<i64> = None;
    for tv in tvs {
        let record = record_of(&mut rig.tw, *tv);
        let due =
            record
                .as_ref()
                .and_then(|record| match (record.act.as_ref(), record.run.as_ref()) {
                    (Some(act), Some(run)) => Some(act.due_ic.min(run.due_ic)),
                    (Some(act), None) => Some(act.due_ic),
                    (None, Some(run)) => Some(run.due_ic),
                    (None, None) => None,
                });
        let wait = tacsim::surface::tick::plan_wait_for_tests(rig.tw.world(), *tv)
            .map(|(_, until)| until)
            .filter(|until| *until > now);
        for at in [due, wait].into_iter().flatten() {
            next = Some(next.map_or(at, |n: i64| n.min(at)));
        }
    }
    next.filter(|at| *at > now).unwrap_or(now + 60)
}

/// **Run the clock until `tvs` have no work** (`plans.rs`'s `work`): each step advances to the
/// next instant a rig needs and lands everything; then two more fires for the last tells.
fn work(rig: &mut Rig, tvs: &[ObjId], max: usize) {
    for _ in 0..max {
        if !tvs.iter().any(|tv| busy(&mut rig.tw, *tv)) {
            let _ = fire(rig);
            let _ = fire(rig);
            return;
        }
        let target = next_instant(rig, tvs);
        let _ = to_ic(rig, target);
    }
    panic!("the rigs still had work after {max} steps");
}

/// Mark `kinds` as the script's while `body` runs (a clerk's or the house's landing the script
/// set up and waits for).
fn expecting<T>(rig: &mut Rig, kinds: &[&str], body: impl FnOnce(&mut Rig) -> T) -> T {
    for kind in kinds {
        rig.expect.insert((*kind).to_owned());
    }
    let out = body(rig);
    for kind in kinds {
        rig.expect.remove(*kind);
    }
    out
}

/// The SM.8 aged-marker seam (`unloading.rs`'s `sm8_the_unload_finish_counts_its_writes`): `n`
/// marked moves of one tonne of `good` out of `from` into `to`, through the inventory's own
/// marked move, each stamped `wall` (real seconds) — `from`'s ring gains one entry each.
fn aged_markers(
    rig: &mut Rig,
    (from, to): (VaultId, VaultId),
    good: &str,
    n: u64,
    (wall, tag): (i64, &str),
) {
    let stamp = rig.tw.world().world_ic_now();
    for k in 0..n {
        let id = mush_inventory::derived_marker_id(1, &json!({ "sm12c.aged": tag, "n": k }))
            .expect("a marker id");
        staff_inventory(rig, |families, txn| {
            families
                .transfer_marked(
                    txn,
                    EndpointRef(from),
                    EndpointRef(to),
                    &cid(good),
                    1,
                    &id,
                    GOD,
                    &stamp,
                    wall,
                )
                .map(|_| ())
        });
    }
}

// ---------------------------------------------------------------------------
// C1 — the write census, on the surface
// ---------------------------------------------------------------------------

/// **C1, the surface** (doc 20 §11's Tick row; the brief's L5): on the shipped lane, two pilots at
/// Ahuna Flank (Ada with two rigs, Bryn with one) through the real verbs and the real ticks — a
/// rig's first launch at the site and a later one; the first reading at the site and a later one;
/// a loud sounding with a listener on the site and a quiet one; a drive; mining runs (a cycle
/// into an empty hopper, the cycles after, the one that works a pocket out) and a run stopped
/// mid-way; recovers near and far (a loaded hopper writing `record.queued`) and the queued unload
/// they file; hand unloads whole and in part; **two goods into two fresh hold pools while the
/// hopper's ring evicts the most it can** (two pushes of eight, SM.8's aged-marker seam stamped
/// two real days back) and **the structural maximum, four goods into four fresh pools with four
/// pushes of eight** (Cass's rig: a hopper holds by mass, so all four ores fit); dumps into a
/// fresh spoil pool and an existing one; a two-family pocket's run (both cursor rows locked, its
/// first cycle into an empty hopper); the chart's forget of a site and of a body with a row at
/// every Ceres site (staff's copies of the pilot's own row); `+tv/quiet` on and off, each reply
/// and its one write asserted. Every kind's maximum within its gate; `done.unload`'s constructed
/// maximum pinned beside SM.8's handed 19.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12c_every_act_kind_counts_its_writes_on_the_surface() {
    let _lock = shared();
    let test = "sm12c_every_act_kind_counts_its_writes_on_the_surface";
    let mut rig = stocked(Roster::default());
    let (ada, ada_row, tvs) = player_on(&mut rig, ("Ada", 1, "Salt Moth"), ("ahuna", AHUNA), 2);
    let (tv1, tv2) = (tvs[0], tvs[1]);
    let (bryn, bryn_row, bryn_tvs) =
        player_on(&mut rig, ("Bryn", 2, "Grey Heron"), ("ahuna", AHUNA), 1);
    let tvb = bryn_tvs[0];
    // The launches: each rig's first at the site.
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv1,
        "+tv/1 launch go",
        "first launch at the site",
    );
    let _ = act_and_finish(
        &mut rig,
        bryn,
        tvb,
        "+tv/1 launch go",
        "first launch at the site",
    );
    // The first reading at the site (a fresh chart record), from the ramp.
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv1,
        "+tv/1 scan go",
        "first reading at the site",
    );
    // A loud sounding with a listener on the site (Bryn's rig is out).
    let stands = stands_in(&mut rig.tw, AHUNA, 'F');
    let (stand, _) = stands[0];
    to_cell_on(&mut rig, AHUNA, tv1, stand);
    let loud = act_and_finish(
        &mut rig,
        ada,
        tv1,
        "+tv/1 sound go",
        "loud, a listener on site",
    );
    // A quiet sounding and a later reading elsewhere.
    let (quiet_stand, _) = stands_in(&mut rig.tw, AHUNA, 'G')[0];
    to_cell_on(&mut rig, AHUNA, tv1, quiet_stand);
    let quiet = act_and_finish(&mut rig, ada, tv1, "+tv/1 sound quiet go", "quiet");
    let _ = act_and_finish(&mut rig, ada, tv1, "+tv/1 scan go", "a later reading");
    let mut pockets: Vec<String> = sounded_pockets(&loud, "metals")
        .into_iter()
        .chain(sounded_pockets(&quiet, "metals"))
        .map(|(addr, _)| addr)
        .collect();
    pockets.dedup();
    assert!(
        !pockets.is_empty(),
        "the soundings named metals:\n{loud}\n{quiet}"
    );
    // A run stopped mid-way, its first cycle into the empty hopper (a fresh hopper pool).
    let mut stopped = None;
    for addr in &pockets {
        rig.variant = "a stopped run: drive".to_owned();
        if try_act(&mut rig, ada, &format!("+tv/1 drive {addr} go")).is_err() {
            continue;
        }
        let _ = finish(&mut rig, tv1, ada);
        rig.variant = "a stopped run: mine".to_owned();
        match try_act(&mut rig, ada, "+tv/1 mine go") {
            Ok(_) => {}
            Err(out) => {
                println!("cost_lane: C1 mine at {addr} refused: {out}");
                continue;
            }
        }
        rig.variant = "mid-run".to_owned();
        let (out, kind, _) = act_go_as(&mut rig, ada, "+tv/1 mine stop", None);
        assert_eq!(kind, "start.mine_stop", "{out}");
        let _ = cycles_named(&mut rig, tv1, ada, "a stopped run, the cycle in flight");
        stopped = Some(addr.clone());
        break;
    }
    let stopped = stopped.expect("a run started and stopped");
    pockets.retain(|addr| *addr != stopped);
    // A run to its end, driven to by hand: the cycles into a loaded hopper, the one that works
    // the pocket out.
    let mined = drive_and_mine(&mut rig, (ada, 1, tv1), &pockets, "a run")
        .expect("a pocket the rig may drive to and mine");
    println!("cost_lane: C1 stopped at {stopped}, mined out {mined}");
    // A far recover with a loaded hopper (it writes `record.queued`), then its queued unload.
    let hop1 = hopper(&rig.tw, tv1);
    let loaded = pool(&mut rig.tw, hop1, "metal_ore");
    assert!(loaded > 0, "the runs loaded the hopper");
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv1,
        "+tv/1 recover go",
        "far, a loaded hopper",
    );
    rig.variant = "queued after a recover".to_owned();
    work(&mut rig, &[tv1], 40);
    assert_eq!(pool(&mut rig.tw, hop1, "metal_ore"), 0);
    // A near recover: rig 2 down the ramp and straight back.
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv2,
        "+tv/2 launch go",
        "first launch at the site",
    );
    let _ = act_and_finish(&mut rig, ada, tv2, "+tv/2 recover go", "near, empty");
    // The hand unloads, in part then whole.
    fill(&mut rig, tv2, "metal_ore", 3, 8_941);
    let _ = act_and_finish(&mut rig, ada, tv2, "+tv/2 unload 1 go", "by hand, in part");
    let _ = act_and_finish(&mut rig, ada, tv2, "+tv/2 unload go", "by hand, whole");
    // Two goods into two fresh hold pools, the hopper's ring evicting the most it can: Bryn's rig
    // home, its hopper filled with both ores, then 48 aged markers at the front of its ring (into
    // Ada's hold, so Bryn's pools stay fresh; staff take them back out of Ada's hold after); the
    // finish's two pushes evict eight each.
    let _ = act_and_finish(&mut rig, bryn, tvb, "+tv/1 recover go", "near, empty");
    let hop_b = hopper(&rig.tw, tvb);
    let ada_hold = hold_vault(&rig.tw, &ada_row);
    let bryn_hold = hold_vault(&rig.tw, &bryn_row);
    fill(&mut rig, tvb, "metal_ore", 51, 8_942);
    fill(&mut rig, tvb, "ice_ore", 1, 8_943);
    let aged = rig.tw.world().now() - 2 * mush_inventory::MARKER_RETENTION_SECS;
    aged_markers(
        &mut rig,
        (hop_b, ada_hold),
        "metal_ore",
        48,
        (aged, "surface"),
    );
    debit_pool(&mut rig, ada_hold, "metal_ore", 48);
    let ring_before = marker_ring(&mut rig.tw, hop_b).map_or(0, |ring| ring.len());
    assert_eq!(ring_before, 48);
    assert_eq!(pool(&mut rig.tw, bryn_hold, "metal_ore"), 0, "a fresh pool");
    assert_eq!(pool(&mut rig.tw, bryn_hold, "ice_ore"), 0, "a fresh pool");
    let _ = act_and_finish(
        &mut rig,
        bryn,
        tvb,
        "+tv/1 unload go",
        "two goods, two fresh pools, two pushes evicting eight each",
    );
    let ring_after = marker_ring(&mut rig.tw, hop_b).map_or(0, |ring| ring.len());
    println!("cost_lane: C1 the evicting unload: ring {ring_before} -> {ring_after}");
    assert_eq!(
        ring_after,
        48 + 2 - 16,
        "two pushes of eight evictions each"
    );
    // **The structural maximum** (the review's correctness/F4): a hopper holds by mass only, so a
    // rig that works a pocket of each family before it comes home carries all four ores; the
    // finish pushes one ring entry per good, each evicting up to `MAX_RING_EVICTIONS_PER_PUSH`.
    // Cass's rig home, its hopper filled with the four ores, 64 aged markers at the front of its
    // ring (so each of the four pushes evicts its eight), into four fresh hold pools.
    let (cass, cass_row, cass_tvs) = player_on(&mut rig, ("Cass", 3, "Tern"), ("ahuna", AHUNA), 1);
    let tvc = cass_tvs[0];
    let _ = act_and_finish(
        &mut rig,
        cass,
        tvc,
        "+tv/1 launch go",
        "first launch at the site",
    );
    let _ = act_and_finish(&mut rig, cass, tvc, "+tv/1 recover go", "near, empty");
    let hop_c = hopper(&rig.tw, tvc);
    let cass_hold = hold_vault(&rig.tw, &cass_row);
    fill(&mut rig, tvc, "metal_ore", 65, 8_946);
    fill(&mut rig, tvc, "ice_ore", 1, 8_947);
    fill(&mut rig, tvc, "carbon_ore", 1, 8_948);
    fill(&mut rig, tvc, "strategic_ore", 1, 8_949);
    let aged = rig.tw.world().now() - 2 * mush_inventory::MARKER_RETENTION_SECS;
    aged_markers(
        &mut rig,
        (hop_c, ada_hold),
        "metal_ore",
        64,
        (aged, "surface-four"),
    );
    debit_pool(&mut rig, ada_hold, "metal_ore", 64);
    assert_eq!(
        marker_ring(&mut rig.tw, hop_c).map_or(0, |ring| ring.len()),
        64
    );
    for (good, _) in ORES {
        assert_eq!(pool(&mut rig.tw, cass_hold, good), 0, "a fresh {good} pool");
    }
    let _ = act_and_finish(
        &mut rig,
        cass,
        tvc,
        "+tv/1 unload go",
        "four goods, four fresh pools, four pushes evicting eight each",
    );
    let ring_four = marker_ring(&mut rig.tw, hop_c).map_or(0, |ring| ring.len());
    println!("cost_lane: C1 the four-goods evicting unload: ring 64 -> {ring_four}");
    assert_eq!(
        ring_four,
        64 + 4 - 4 * mush_inventory::MAX_RING_EVICTIONS_PER_PUSH,
        "four pushes of eight evictions each"
    );
    for (good, _) in ORES {
        assert_eq!(pool(&mut rig.tw, hop_c, good), 0, "the {good} unloaded");
    }
    // The dumps: a fresh spoil pool, then the same one.
    let _ = act_and_finish(&mut rig, ada, tv2, "+tv/2 launch go", "a later launch");
    fill(&mut rig, tv2, "metal_ore", 2, 8_944);
    let _ = act_and_finish(&mut rig, ada, tv2, "+tv/2 dump go", "a fresh spoil pool");
    fill(&mut rig, tv2, "metal_ore", 2, 8_945);
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv2,
        "+tv/2 dump go",
        "an existing spoil pool",
    );
    // A two-family pocket (the brief's L5; the review's tests/T1): its run locks both families'
    // cursor rows, its first cycle into an empty hopper. Rig 2, emptied by the dumps, on the
    // first such pocket of the keyed truth the census has not touched, sounded quietly there.
    let (two, two_stand) = two_family_pocket(&mut rig.tw, AHUNA, &[stopped.clone(), mined.clone()]);
    to_cell_on(&mut rig, AHUNA, tv2, two_stand);
    recharge(&mut rig, tv2);
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv2,
        "+tv/2 sound quiet go",
        "at a two-family pocket",
    );
    rig.variant = "a two-family pocket".to_owned();
    let (out, kind, writes) = act_go_as(&mut rig, ada, "+tv/2 mine go", None);
    assert_eq!(kind, "start.mine", "{out}");
    let locks = record_of(&mut rig.tw, tv2)
        .and_then(|record| record.run)
        .map(|run| run.lock_rows.len());
    println!(
        "cost_lane: C1 a two-family pocket {two}: start.mine writes {writes}, locks {locks:?}"
    );
    assert_eq!(locks, Some(2), "both families' rows locked");
    let _ = cycles_named(&mut rig, tv2, ada, "a two-family pocket");
    // The chart's forget at its most rows (the review's conformance/F6, tests/T1): a body's forget
    // stages one write per site row the body holds (`ChartForgetProvider`, one per site the
    // target names). Staff copy Ada's Ahuna row to the other four Ceres sites — the most rows a
    // forget of Ceres can name; a real verb reads one site a hop — then `+tv/forget ceres`.
    let ahuna_row = chart_of(&mut rig.tw, ada, AHUNA).expect("Ada's Ahuna row");
    for (site, _) in CERES_SITES {
        if site != AHUNA {
            put_chart(
                &mut rig,
                tacsim::surface::ChartRecord {
                    site: cid(site),
                    ..ahuna_row.clone()
                },
            );
        }
    }
    rig.variant = "a body's charts, a row at every Ceres site".to_owned();
    let (out, kind, writes) = act_go_as(&mut rig, ada, "+tv/forget ceres go", None);
    assert_eq!(kind, "forget", "{out}");
    println!("cost_lane: C1 the body's forget writes {writes}: {out}");
    // A site's forget: a fresh reading at Ahuna first.
    let _ = act_and_finish(&mut rig, ada, tv2, "+tv/2 scan go", "a later reading");
    rig.variant = "a site's chart".to_owned();
    let (out, kind, _) = act_go_as(&mut rig, ada, "+tv/forget ahuna flank go", None);
    assert_eq!(kind, "forget", "{out}");
    // The preference document: its first write and a later one, each the verb's own reply and
    // one write (the figure O-24b rests on: the review's tests/T5).
    rig.variant = "first write".to_owned();
    let (out, writes) = normal_cmd(&mut rig, ada, "+tv/quiet on", "tvpref");
    println!("cost_lane: C1 +tv/quiet on: {out}");
    assert!(out.contains(tacsim::port::tv::QUIET_ON), "{out}");
    assert_eq!(writes, 1, "the preference document's first write");
    rig.variant = "a later write".to_owned();
    let (out, writes) = normal_cmd(&mut rig, ada, "+tv/quiet off", "tvpref");
    assert!(out.contains(tacsim::port::tv::QUIET_OFF), "{out}");
    assert_eq!(writes, 1, "a later write of the preference document");
    census_close(&rig, test);
    assert_black_box_pair(&rig, test);
    let unload = constructed(&rig, "done.unload");
    println!(
        "cost_lane: C1 done.unload SM.8 19 handed / SM.12c constructed {unload} (O-20); variants \
         {:?}",
        rig.tally.kinds.get("done.unload").map(|row| &row.variants)
    );
}

// ---------------------------------------------------------------------------
// C1 — the write census, in a plan
// ---------------------------------------------------------------------------

/// **C1, a plan** (doc 20 §8 "Leaving it running"; the brief's L5): on the shipped lane, a pilot
/// at Ahuna Flank scans by hand and files a fill plan over the blocks it read (`start.plan_file`),
/// which runs to its end — its drives, soundings, runs and recovers as `start.<act>.plan`, the
/// queued unloads after each `Done` recover, and `start.plan_end` with its `by`; the ended plan is
/// struck (`start.plan_strike`); a second plan filed and cleared while live
/// (`start.plan_clear`); and a rig left idle on the ground comes home after 24 IC hours
/// (`start.recover.idle`), its loaded hopper unloading after.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12c_every_act_kind_counts_its_writes_in_a_plan() {
    let _lock = shared();
    let test = "sm12c_every_act_kind_counts_its_writes_in_a_plan";
    let mut rig = stocked(Roster::default());
    let (ada, row, tvs) = player_on(&mut rig, ("Ada", 1, "Salt Moth"), ("ahuna", AHUNA), 2);
    let (tv1, tv2) = (tvs[0], tvs[1]);
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv1,
        "+tv/1 launch go",
        "first launch at the site",
    );
    let (stand, _) = stands_in(&mut rig.tw, AHUNA, 'F')[0];
    to_cell_on(&mut rig, AHUNA, tv1, stand);
    let _ = act_and_finish(
        &mut rig,
        ada,
        tv1,
        "+tv/1 scan go",
        "a hand scan for the plan",
    );
    let chart = chart_of(&mut rig.tw, ada, AHUNA).expect("the pilot's chart");
    let mut letters: Vec<char> = chart
        .fields
        .keys()
        .filter_map(|index| {
            let (fx, fy) = tacsim::surface::address::field_of_index(*index)?;
            tacsim::surface::address::letter_of_block(fx / 2, fy / 2)
        })
        .collect();
    letters.sort_unstable();
    letters.dedup();
    let list: Vec<String> = letters.iter().map(char::to_string).collect();
    // The plan, filed (the preview first: it commits nothing).
    let line = format!("+tv/1 plan {} family=metal_ore", list.join(","));
    let (preview, queued) = command(&mut rig, ada, &line);
    assert!(queued.is_empty(), "{preview}");
    println!("cost_lane: C1 plan preview:\n{preview}");
    rig.variant = "a plan over the scanned blocks".to_owned();
    let (out, kind, _) = act_go_as(&mut rig, ada, &format!("{line} go"), None);
    assert_eq!(kind, "start.plan_file", "{out}");
    rig.variant = "the plan's steps".to_owned();
    work(&mut rig, &[tv1], 3_000);
    let plan = record_of(&mut rig.tw, tv1)
        .and_then(|record| record.plan)
        .expect("the plan");
    println!(
        "cost_lane: C1 the plan ended {:?} {:?}",
        plan.state, plan.tally
    );
    // `start.plan_end` keeps its `by` (the brief's L4.1; the review's tests/T1): the tick's end of
    // a player's plan, `by: plan` (`surface::tick`'s plan-end submission), never another.
    assert_eq!(
        rig.tally
            .plan_end_by
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["plan"],
        "start.plan_end's by"
    );
    // The ended plan struck.
    rig.variant = "an ended plan struck".to_owned();
    let (out, kind, _) = act_go_as(&mut rig, ada, "+tv/1 plan clear", None);
    assert_eq!(kind, "start.plan_strike", "{out}");
    // A second plan, filed and cleared while live (rig 2, from the bay): staff empty the hold of
    // the first plan's ore first, so a cycle fits.
    let hold = hold_vault(&rig.tw, &row);
    let ore = pool(&mut rig.tw, hold, "metal_ore");
    debit_pool(&mut rig, hold, "metal_ore", ore);
    let (out, queued) = command(
        &mut rig,
        ada,
        &format!("+tv/2 plan {} family=metal_ore go", list.join(",")),
    );
    assert_eq!(queued.len(), 1, "{out}");
    rig.variant = "a plan filed from the bay".to_owned();
    let _ = land_all(&mut rig, &queued);
    rig.variant = "a live plan cleared".to_owned();
    let (out, kind, _) = act_go_as(&mut rig, ada, "+tv/2 plan clear", None);
    assert_eq!(kind, "start.plan_clear", "{out}");
    rig.variant = "the cleared plan's last step".to_owned();
    work(&mut rig, &[tv2], 400);
    // The idle return: rig 1 out on the ground with ore, left 24 IC hours with no order.
    let _ = act_and_finish(&mut rig, ada, tv1, "+tv/1 launch go", "a later launch");
    fill(&mut rig, tv1, "metal_ore", 2, 8_951);
    rig.variant = "the idle return".to_owned();
    let idle_at = ic_now(&rig.tw) + tacsim::surface::plan::IDLE_PARK_IC_SECS + 60;
    let _ = to_ic(&mut rig, idle_at);
    for _ in 0..4 {
        let _ = fire(&mut rig);
    }
    rig.variant = "queued after the idle return".to_owned();
    work(&mut rig, &[tv1], 40);
    census_close(&rig, test);
    assert_black_box_pair(&rig, test);
}

/// **Doc 23 cut S.1: the black box pair** (doc 23 §6, §9's S.1 row "the SM.12c cost lane re-run
/// with the two writes per excursion inside its budget"): the launch's finish and the recover's
/// each carried one `log:hull.<n>` chunk in their bundles — counted in `done.launch`'s and
/// `done.recover`'s writes, both still inside their §11 ceilings ([`census_close`] asserts it) —
/// and no kind other than a finish that leaves the bay (a launch, or a scan, a sounding or a
/// drive from it) or a recover's ever wrote one.
fn assert_black_box_pair(rig: &Rig, test: &str) {
    println!(
        "cost_lane: C1 {test} black box lines by kind {:?}",
        rig.tally.black_box
    );
    for kind in ["done.launch", "done.recover"] {
        assert!(
            rig.tally.black_box.get(kind).is_some_and(|n| *n > 0),
            "{test}: {kind} carries its black box line: {:?}",
            rig.tally.black_box
        );
    }
    // Doc 23 cut S.2: the ship's epoch (the port tick's `tacsim_voyage_epoch`, a kind the gates
    // leave `other.*`) writes the Dray-P's own pair.
    let allowed = [
        "done.launch",
        "done.scan",
        "done.sound",
        "done.drive",
        "done.recover",
        "other.tacsim_voyage_epoch",
    ];
    assert!(
        rig.tally
            .black_box
            .keys()
            .all(|kind| allowed.contains(&kind.as_str())),
        "{test}: only an epoch's open or close writes a black box line: {:?}",
        rig.tally.black_box
    );
}

// ---------------------------------------------------------------------------
// The tick-borne normal-lane writes, attributed by key (the brief's L4.1 and L3.9)
// ---------------------------------------------------------------------------

/// Debit `qty` of `good` from `vault`'s pool (staff's hand, the inventory's own path).
fn debit_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64) {
    if qty == 0 {
        return;
    }
    staff_inventory(rig, |families, txn| {
        families
            .debit(txn, EndpointRef(vault), &cid(good), qty)
            .map(|_| ())
    });
}

/// **What the port's normal-lane steps write, read before and after a fire**: the Ceres map, its
/// vendors' asks (listing id and quantity by org and good), whether Occator Rim's yard holds
/// the house's licence, and the economy sweep's stamps (doc 36 cut GC.7: a fire the economy's
/// `Coarse` sweep also ran in carries its `economy:meta` stamp, which is the sweep's write and
/// no clerk's).
#[derive(Clone, Debug, Default)]
struct TickRead {
    ceres: Option<PortMap>,
    asks: BTreeMap<(String, String), Option<(u64, u64)>>,
    house_licence: bool,
    sweep: Option<(i64, i64)>,
}

/// The Ceres map, if present.
fn ceres_map(tw: &TestWorld) -> Option<PortMap> {
    match store(tw).port_map(&cid(LANE_PACK), &cid("ceres_port")) {
        PortMapState::Present(map) => Some(map.clone()),
        _ => None,
    }
}

/// Read what [`TickRead`] names.
fn tick_read(tw: &TestWorld) -> TickRead {
    let ceres = ceres_map(tw);
    let mut asks = BTreeMap::new();
    if let (Some(map), Some(market)) = (
        ceres.as_ref(),
        tw.world().module_state::<mush_market::MarketStore>(),
    ) {
        for (org, row) in &map.vendors {
            for stock in &row.stock {
                let listing = market
                    .lot_listing_of(EndpointRef(VaultId(row.vault)), &stock.good)
                    .and_then(|id| market.listing(id).map(|l| (id.0, l.quantity)));
                asks.insert((org.to_string(), stock.good.to_string()), listing);
            }
        }
    }
    let house_licence = match store(tw).port_map(&cid(LANE_PACK), &cid(OCCATOR)) {
        PortMapState::Present(map) => map
            .site
            .as_ref()
            .and_then(|site| site.yard.as_ref())
            .is_some_and(|yard| yard.licence.is_some()),
        _ => false,
    };
    TickRead {
        ceres,
        asks,
        house_licence,
        sweep: tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .map(mush_economy::EconomyStore::last_sweep),
    }
}

/// **The Ceres clerk's act in one fire** (doc 20a §5.6, CMP-9; the brief's L3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ClerkAct {
    /// An emergency bid posted or moved (`PortMap.emergencies`).
    EmergencyBid,
    /// A mill product's ask raised by the units milled (`mill.restock`).
    Restock,
    /// A mill batch landed for the Ceres clerk (`mill.batch`).
    Batch,
    /// An ore bid's intake drip (its `posted_ic` moved).
    Drip,
    /// Another bid row's listing re-posted.
    BidRepost,
    /// An ask's listing id appeared or changed (`other.list`; SM.12a's relisting).
    List,
}

impl ClerkAct {
    fn word(self) -> &'static str {
        match self {
            ClerkAct::EmergencyBid => "emergency_bid",
            ClerkAct::Restock => "restock",
            ClerkAct::Batch => "batch",
            ClerkAct::Drip => "drip",
            ClerkAct::BidRepost => "bid_repost",
            ClerkAct::List => "list",
        }
    }
}

/// The ore goods a vendor bids for (the drip's rows).
fn is_ore(good: &CanonicalId) -> bool {
    good.as_str().ends_with("_ore")
}

/// **The Ceres clerk's step reader** (the brief's L3.9), over the Ceres map and its asks read
/// before and after a port fire, in this precedence (first match wins): `Batch` when a
/// `mill.batch` landed for the Ceres clerk; `List` when an ask's listing id appeared or changed
/// (even when its mill counter moved too: SM.12a's 3-write relisting); `Restock` only when a
/// `mill_listed` counter rose **and** the same listing's quantity rose; `EmergencyBid` when the
/// emergencies changed; `Drip` when an ore bid row's `posted_ic` moved; `BidRepost` when another
/// bid row's listing moved. With the act, the market listing ids it touched.
fn clerk_act(before: &TickRead, after: &TickRead, batch: bool) -> Option<(ClerkAct, Vec<u64>)> {
    if batch {
        return Some((ClerkAct::Batch, Vec::new()));
    }
    let (Some(old), Some(new)) = (before.ceres.as_ref(), after.ceres.as_ref()) else {
        return None;
    };
    let listed: Vec<u64> = after
        .asks
        .iter()
        .filter_map(|(key, now)| {
            let was = before.asks.get(key).copied().flatten();
            match (was, *now) {
                (Some((a, _)), Some((b, _))) if a != b => Some(b),
                (None, Some((b, _))) => Some(b),
                _ => None,
            }
        })
        .collect();
    if !listed.is_empty() {
        return Some((ClerkAct::List, listed));
    }
    let mut restocked = Vec::new();
    for (org, row) in &new.vendors {
        let Some(was) = old.vendors.get(org) else {
            continue;
        };
        if row.mill_listed == was.mill_listed {
            continue;
        }
        for stock in &row.stock {
            let key = (org.to_string(), stock.good.to_string());
            if let (Some(Some((a, qa))), Some(Some((b, qb)))) =
                (before.asks.get(&key), after.asks.get(&key))
                && a == b
                && qb > qa
            {
                restocked.push(*b);
            }
        }
    }
    if !restocked.is_empty() {
        return Some((ClerkAct::Restock, restocked));
    }
    if new.emergencies != old.emergencies {
        let ids = new.emergencies.values().map(|row| row.listing).collect();
        return Some((ClerkAct::EmergencyBid, ids));
    }
    let mut drips = Vec::new();
    let mut reposts = Vec::new();
    for (org, row) in &new.vendors {
        let Some(was) = old.vendors.get(org) else {
            continue;
        };
        for (good, bid) in &row.bids {
            let Some(prior) = was.bids.get(good) else {
                continue;
            };
            if is_ore(good) && bid.posted_ic != prior.posted_ic {
                drips.push(bid.listing);
            } else if bid.listing != prior.listing {
                reposts.push(bid.listing);
            }
        }
    }
    if !drips.is_empty() {
        return Some((ClerkAct::Drip, drips));
    }
    if !reposts.is_empty() {
        return Some((ClerkAct::BidRepost, reposts));
    }
    None
}

/// **Attribute one fire's tick-borne normal-lane writes** (the brief's L4.1): the Ceres clerk's
/// act read by [`clerk_act`] — a restock is `mill.restock`, every other step `other.<step>`
/// (the drip, the emergency bid, the bid re-post, the relisting: counted and reported, never
/// gated, O-24), each counted from the fire's own ops on its listing keys, `market:meta` and the
/// Ceres map key — and the house's licence item (`site.house_licence`: the fire in which the
/// yard's licence appeared, counted from its economy ops and Occator Rim's map key). The act, if
/// any.
fn tick_writes(
    rig: &mut Rig,
    (before, after): (&TickRead, &TickRead),
    ops: &[OpKey],
    landed: &[(String, ObjId)],
) -> Option<ClerkAct> {
    let ceres_clerk = after
        .ceres
        .as_ref()
        .and_then(|map| map.clerk.as_ref())
        .map(|clerk| ObjId(clerk.thing));
    let batch = landed
        .iter()
        .any(|(kind, actor)| kind == "mill.batch" && Some(*actor) == ceres_clerk);
    let ceres_key = tacsim::port::port_map_key(&cid(LANE_PACK), &cid("ceres_port"));
    let act = clerk_act(before, after, batch);
    if let Some((act, listings)) = &act
        && *act != ClerkAct::Batch
    {
        let keys: BTreeSet<String> = listings
            .iter()
            .map(|id| format!("market:listing:{id}"))
            .collect();
        // Distinct keys: `market:meta` is written by every listing any clerk makes in the fire,
        // so it counts once, and only for a step that makes a listing.
        let meta = matches!(
            act,
            ClerkAct::List | ClerkAct::EmergencyBid | ClerkAct::Drip | ClerkAct::BidRepost
        );
        let counted: BTreeSet<&str> = ops
            .iter()
            .filter(|op| {
                op.key == ceres_key
                    || keys.contains(op.key.as_str())
                    || (meta && op.key.as_str() == mush_market::META_KEY)
            })
            .map(|op| op.key.as_str())
            .collect();
        let writes = counted.len();
        let kind = match act {
            ClerkAct::Restock => "mill.restock".to_owned(),
            other => format!("other.{}", other.word()),
        };
        let constructed = rig.expect.contains(&kind).then(|| rig.variant.clone());
        rig.tally.landed(
            (&kind, &format!("normal:{kind}")),
            Who::Clerk,
            writes,
            0,
            constructed.as_deref(),
        );
        if kind == "mill.restock" || kind == "other.list" {
            println!(
                "cost_lane: tick {kind} writes {writes}: {:?}",
                ops.iter()
                    .filter(|op| op.key == ceres_key
                        || keys.contains(op.key.as_str())
                        || op.key.as_str() == mush_market::META_KEY)
                    .map(|op| op.key.to_string())
                    .collect::<Vec<_>>()
            );
        }
        if constructed.is_some() {
            println!(
                "cost_lane: tick {kind} the fire's every op: {:?}",
                ops.iter()
                    .map(|op| format!("{}:{}", op.ns, op.key))
                    .collect::<Vec<_>>()
            );
        }
        // **The census's restock fire is the restock's alone** (the review's tests/T4: a count
        // over the expected keys cannot see an added key or a doubled write): where the census
        // constructs it, every op the fire wrote is one the restock counted, once.
        if kind == "mill.restock" && constructed.is_some() {
            // The economy sweep's own stamp, exactly when the sweep ran in this fire.
            let swept = before.sweep != after.sweep;
            let sweep_stamp = ops
                .iter()
                .filter(|op| swept && op.ns == "economy" && op.key == mush_economy::META_KEY)
                .count();
            assert!(sweep_stamp <= 1, "one sweep stamp a fire: {ops:?}");
            assert_eq!(
                ops.len() - sweep_stamp,
                writes,
                "the census's restock fire wrote beyond its counted keys: {ops:?}"
            );
        }
    }
    if after.house_licence && !before.house_licence {
        // The handler's three keys (SM.12b): the house's licence account, the economy's meta and
        // Occator Rim's map — each counted once (the economy's meta is any account opening's).
        let occator_key = tacsim::port::port_map_key(&cid(LANE_PACK), &cid(OCCATOR));
        let account = match store(&rig.tw).port_map(&cid(LANE_PACK), &cid(OCCATOR)) {
            PortMapState::Present(map) => map
                .site
                .as_ref()
                .and_then(|site| site.yard.as_ref())
                .and_then(|yard| yard.licence.as_ref())
                .map(|licence| format!("account:{}", licence.account)),
            _ => None,
        };
        let counted: BTreeSet<String> = ops
            .iter()
            .filter(|op| {
                op.key == occator_key
                    || (op.ns.as_str() == "economy"
                        && (op.key.as_str() == "meta"
                            || Some(op.key.as_str()) == account.as_deref()))
            })
            .map(|op| format!("{}:{}", op.ns, op.key))
            .collect();
        println!(
            "cost_lane: tick site.house_licence writes {}: {counted:?}",
            counted.len()
        );
        let kind = "site.house_licence";
        let constructed = rig.expect.contains(kind).then(|| rig.variant.clone());
        if constructed.is_some() {
            println!(
                "cost_lane: tick {kind} the fire's every op: {:?}",
                ops.iter()
                    .map(|op| format!("{}:{}", op.ns, op.key))
                    .collect::<Vec<_>>()
            );
            // As the restock's (the review's tests/T4): the census's licence fire is the
            // handler's alone, every op one it counted, once.
            assert_eq!(
                ops.len(),
                counted.len(),
                "the census's house-licence fire wrote beyond its counted keys: {ops:?}"
            );
        }
        rig.tally.landed(
            (kind, "normal:site.house_licence"),
            Who::Clerk,
            counted.len(),
            0,
            constructed.as_deref(),
        );
    }
    act.map(|(act, _)| act)
}

// ---------------------------------------------------------------------------
// The kit and the hop (the census's; the month bunkers at the market instead)
// ---------------------------------------------------------------------------

/// **A licensed pilot with `n_rigs` rigs leased**, 20 t of propellant and provisions seeded into
/// the hold by staff, at Ceres (not yet aboard). The licence and each lease counted under their
/// variants.
fn kit_up(rig: &mut Rig, (who, seed, name): (&str, i64, &str), n_rigs: usize) -> (ObjId, HullRow) {
    rig.variant = "the first licence on an account".to_owned();
    let (pilot, row) = prospector(rig, who, seed, name);
    for n in 0..n_rigs {
        rig.variant = if n == 0 {
            "the first lease"
        } else {
            "a second rig"
        }
        .to_owned();
        let _ = lease_one(rig, pilot);
    }
    let vault = hold_vault(&rig.tw, &row);
    let lot = 9_100 + u64::from(row.hull) * 10;
    seed_pool(rig, vault, "propellant", 20, LotId(lot));
    seed_pool(rig, vault, "provisions", 20, LotId(lot + 1));
    (pilot, row)
}

/// **The hop**: `pilot`'s hull from Ceres to `entry`'s pad (the site opened first), settled, the
/// site warmed.
fn fly_to(rig: &mut Rig, pilot: ObjId, row: &HullRow, (venue, entry): (&str, &str)) {
    let hull = ObjId(row.hull);
    open_site(rig, entry);
    let arrives = file_and_launch(rig, pilot, hull, &format!("+ship/undock {venue}"));
    let _ = to_ic(rig, arrives);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == entry
        ),
        "the hull is settled on {entry}'s pad"
    );
    warm(rig, entry);
}

/// **A licensed pilot's Dray-P settled on `entry`'s pad**, `n_rigs` rigs leased, the pilot
/// aboard and connected ([`kit_up`] then [`fly_to`]).
fn player_on(
    rig: &mut Rig,
    pilot_names: (&str, i64, &str),
    (venue, entry): (&str, &str),
    n_rigs: usize,
) -> (ObjId, HullRow, Vec<ObjId>) {
    let (pilot, row) = kit_up(rig, pilot_names, n_rigs);
    board(rig, pilot);
    fly_to(rig, pilot, &row, (venue, entry));
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), n_rigs);
    (pilot, row, tvs)
}

// ---------------------------------------------------------------------------
// C1 — the write census, in tenure
// ---------------------------------------------------------------------------

/// A block's public tenure record.
fn block_record(
    tw: &mut TestWorld,
    entry: &str,
    letter: char,
) -> Option<tacsim::surface::claims::BlockTenure> {
    let claims = handles(tw).families.claims;
    typed(
        tw,
        &claims,
        &tacsim::surface::claims::BlockKey {
            site: cid(entry),
            letter,
        },
    )
}

/// The block's claim, if one stands.
fn block_claim(
    tw: &mut TestWorld,
    entry: &str,
    letter: char,
) -> Option<tacsim::surface::claims::BlockClaim> {
    block_record(tw, entry, letter).and_then(|record| record.claim)
}

/// Write `record` into `tacsim.claims` through a typed transaction (staff's hand; `claims.rs`'s
/// `put_block`).
fn put_block(rig: &mut Rig, record: tacsim::surface::claims::BlockTenure) {
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
    keep(rig, &ops);
}

/// Put `tv` on a soft stand of block `letter` (staff's placement) and scan there by hand: the
/// pilot's own reading of the block.
fn scanned_in(rig: &mut Rig, entry: &str, (pilot, slot, tv): (ObjId, u8, ObjId), letter: char) {
    let last = chart_of(&mut rig.tw, pilot, entry).and_then(|row| row.scan_stand);
    let stand = stands_in(&mut rig.tw, entry, letter)
        .into_iter()
        .map(|(stand, _)| stand)
        .find(|(x, y)| last != Some([*x, *y]))
        .unwrap_or_else(|| panic!("no fresh soft stand in block {letter}"));
    to_cell_on(rig, entry, tv, stand);
    recharge(rig, tv);
    let _ = act_and_finish(
        rig,
        pilot,
        tv,
        &format!("+tv/{slot} scan go"),
        "a hand reading",
    );
}

/// `+claim/… go` as `who` under `label`: exactly one tenure act queued, landed; the reply.
fn claim_go(rig: &mut Rig, who: ObjId, line: &str, label: Option<&'static str>) -> String {
    let (out, kind, _) = act_go_as(rig, who, line, label);
    assert!(kind.starts_with("claim."), "{line}: {kind}: {out}");
    out
}

/// Fire (six-second fires, everything landed) `n` times; what `who` heard.
fn fires(rig: &mut Rig, n: usize, who: ObjId) -> String {
    let mut heard = String::new();
    for _ in 0..n {
        let events = fire(rig);
        heard.push_str(&said(&rig.tw, &events, who));
        heard.push('\n');
    }
    heard
}

/// **Work at least `tonnes` of block `letter`** by real cycles (`claims.rs`'s `work`): sound
/// quietly at a stand, mine it out run by run, dump between hoppers, move to the next stand; the
/// battery recharged by staff's hand between runs (SM.10's seam). The tonnes worked.
fn work_block(
    rig: &mut Rig,
    entry: &str,
    (pilot, slot, tv): (ObjId, u8, ObjId),
    letter: char,
    tonnes: u64,
) -> u64 {
    let drawn = |rig: &mut Rig| -> u64 { worked_now(&mut rig.tw, entry, letter) };
    let start = drawn(rig);
    for (stand, _) in stands_in(&mut rig.tw, entry, letter) {
        if drawn(rig) - start >= tonnes {
            break;
        }
        to_cell_on(rig, entry, tv, stand);
        recharge(rig, tv);
        rig.variant = "on its own claimed block".to_owned();
        let _ = act_and_finish(
            rig,
            pilot,
            tv,
            &format!("+tv/{slot} sound quiet go"),
            "quiet",
        );
        for _ in 0..8 {
            if drawn(rig) - start >= tonnes {
                break;
            }
            recharge(rig, tv);
            let before = drawn(rig);
            rig.variant = "on its own claimed block".to_owned();
            if try_act(rig, pilot, &format!("+tv/{slot} mine go")).is_err() {
                break;
            }
            let _ = cycles_named(rig, tv, pilot, "on its own claimed block");
            let _ = act_and_finish(
                rig,
                pilot,
                tv,
                &format!("+tv/{slot} dump go"),
                "between hoppers",
            );
            if drawn(rig) == before {
                break;
            }
        }
    }
    let worked = drawn(rig) - start;
    assert!(worked >= tonnes, "worked {worked} of {tonnes} t");
    worked
}

/// The tonnes drawn from block `letter`'s deposits (their pocket cursors' totals).
fn worked_now(tw: &mut TestWorld, entry: &str, letter: char) -> u64 {
    let pockets = handles(tw).families.pockets;
    tacsim::surface::geology::OreFamily::ALL
        .into_iter()
        .filter_map(|family| {
            mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
                &cid(entry),
                letter,
                family,
            ))
            .ok()
        })
        .filter_map(|id| typed(tw, &pockets, &id))
        .filter_map(|cursor| cursor.drawn_total())
        .sum()
}

/// **An outage past a fee's real hour** (`claims.rs`'s): one leap of the clock at world rate 1,
/// everything due fired once, then whatever the leap queued landed; the rate restored.
fn outage(rig: &mut Rig) {
    set_rate(&mut rig.tw, 1);
    sweep(rig, tacsim::surface::tenure::CLAIM_FEE_WINDOW_SECS + 60);
    set_rate(&mut rig.tw, RATE);
}

/// **C1, tenure** (doc 20 §9; SM.10; the brief's L5): on the shipped lane, the clerks' opening
/// acts at a fresh boot (`site.open`, `site.worked`, Occator Rim's held seeds `claim.seed`), the
/// licence (the first on an account and a renewal), the lease (the first, a second rig, a rig
/// returned), and every claim act: the first stake at a site, a stake on Occator Rim's papered
/// block, a re-stake of a reopened block; the release; the contest won by a heard sounding inside
/// the epoch; the renewal past the worked floor, reached by real cycles on the claimed block; the
/// unpaid fee's two variants — withdrawn after landing, and expired unaccepted (SM.10's seam);
/// the term lapse (SM.10's `to_ic` seam). Plus the unlabelled-claim guard on the pure core.
/// The claim acts run at Kerwan (`KERWAN`), whose blocks hold up to four deposits: the largest
/// stake, contest and release the lane has (the month's first smoke found them over the
/// Ahuna-built census; the gates were re-pinned here).
#[test]
#[allow(clippy::too_many_lines)]
fn sm12c_every_act_kind_counts_its_writes_in_tenure() {
    let _lock = shared();
    let test = "sm12c_every_act_kind_counts_its_writes_in_tenure";
    let mut rig = stocked_expecting(
        Roster::default(),
        &["site.open", "site.worked", "claim.seed"],
        "a fresh boot's first fires",
    );
    // The clerks' opening acts, at a fresh boot's first fires.
    rig.variant = "a fresh boot's first fires".to_owned();
    expecting(
        &mut rig,
        &["site.open", "site.worked", "claim.seed"],
        |rig| {
            open_site(rig, KERWAN);
            rig.variant = "Occator Rim's held seeds".to_owned();
            open_site(rig, OCCATOR);
            rig.variant = "every other lane site".to_owned();
            for entry in SITES {
                open_site(rig, entry);
            }
            for _ in 0..30 {
                let _ = fire(rig);
            }
        },
    );
    // The site clerk's worked-share transfers stay the script's for the rest of the test: the
    // largest moves the share of a block the script's own pilots mined (Bryn's renewal below).
    rig.expect.insert("site.worked".to_owned());
    // The licence and the lease: Ada's first licence and two leases, the second rig returned at
    // once, then a renewal of the licence.
    let (ada, ada_row) = kit_up(&mut rig, ("Ada", 1, "Salt Moth"), 2);
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, ada, counter);
    rig.variant = "a rig in the bay, its hopper empty".to_owned();
    let (out, _) = normal_cmd(&mut rig, ada, "+tv/2 lease return go", "lease.return");
    println!("cost_lane: C1 lease return: {out}");
    rig.variant = "a renewal in force".to_owned();
    let (out, _) = normal_cmd(&mut rig, ada, "+licence/buy go", "licence.buy");
    assert!(
        out.contains("licenses you to prospect") || out.contains("extends your licence"),
        "{out}"
    );
    board(&mut rig, ada);
    fly_to(&mut rig, ada, &ada_row, ("kerwan", KERWAN));
    let ta = rigs_of(&rig.tw, ada)[0];
    let (bryn, _, bryn_tvs) = player_on(&mut rig, ("Bryn", 2, "Grey Heron"), ("kerwan", KERWAN), 1);
    let tb = bryn_tvs[0];
    let _ = act_and_finish(
        &mut rig,
        ada,
        ta,
        "+tv/1 launch go",
        "first launch at the site",
    );
    let _ = act_and_finish(
        &mut rig,
        bryn,
        tb,
        "+tv/1 launch go",
        "first launch at the site",
    );
    // The blocks, by their opened deposits (a stake files a claim on every deposit of its block, so
    // the block with the most is the stake's and the release's structural worst case, which the
    // month found above the first census's figures): the richest for the first stake, the release
    // and the renewal; the richest at least two blocks from it for the contest; three more.
    let ranked = blocks_by_deposits(&mut rig.tw, KERWAN);
    let big = ranked[0];
    let far = ranked
        .iter()
        .copied()
        .find(|letter| block_distance(big, *letter) >= 2)
        .expect("a block two blocks from the richest");
    let rest: Vec<char> = ranked
        .iter()
        .copied()
        .filter(|letter| *letter != big && *letter != far)
        .collect();
    let (withdrawn, never, expired_block) = (rest[0], rest[1], rest[2]);
    let papered = blocks_by_deposits(&mut rig.tw, OCCATOR)
        .into_iter()
        .find(|letter| !['F', 'G', 'J', 'K'].contains(letter))
        .expect("an outer block");
    println!(
        "cost_lane: C1 tenure blocks: big {big}, far {far}, withdrawn {withdrawn}, never {never}, \n         expired {expired_block}, papered {papered}; deposits {:?}",
        ranked
            .iter()
            .map(|letter| (*letter, opened_deposits(&mut rig.tw, KERWAN, *letter)))
            .collect::<Vec<_>>()
    );
    // The first stake at the site: Ada reads the richest block by hand and stakes it.
    scanned_in(&mut rig, KERWAN, (ada, 1, ta), big);
    rig.variant = "the first claim at the site".to_owned();
    let out = claim_go(&mut rig, ada, "+claim/stake go", Some("stake"));
    assert!(out.contains(&format!("You stake block {big} at ")), "{out}");
    let _ = fires(&mut rig, 12, ada);
    // The contest: Bryn's loud sounding of D heard first, then Ada reads D and stakes it, then
    // Bryn contests it from aboard his hull on the site.
    let (stand, _) = stands_in(&mut rig.tw, KERWAN, far)[0];
    to_cell_on(&mut rig, KERWAN, tb, stand);
    recharge(&mut rig, tb);
    let _ = act_and_finish(&mut rig, bryn, tb, "+tv/1 sound go", "loud, the contest's");
    scanned_in(&mut rig, KERWAN, (ada, 1, ta), far);
    rig.variant = "a second claim, to be contested".to_owned();
    let out = claim_go(&mut rig, ada, "+claim/stake go", Some("stake"));
    assert!(out.contains(&format!("You stake block {far} at ")), "{out}");
    for _ in 0..40 {
        let fee = block_claim(&mut rig.tw, KERWAN, far).and_then(|claim| claim.fee);
        let settled = fee.is_some_and(|fee| {
            matches!(
                rig.tw
                    .world()
                    .module_state::<mush_economy::EconomyStore>()
                    .and_then(|e| e
                        .snapshot()
                        .contract(mush_economy::ContractId(fee))
                        .map(|c| c.state)),
                Some(mush_economy::ContractState::Settled { .. })
            )
        });
        if settled {
            break;
        }
        sweep(&mut rig, 60);
    }
    rig.variant = "won by a heard sounding inside the epoch".to_owned();
    let out = claim_go(
        &mut rig,
        bryn,
        &format!("+claim/contest {far} go"),
        Some("contest"),
    );
    assert!(
        out.contains(&format!("You contest block {far} at Kerwan Floor;")),
        "{out}"
    );
    let _ = fires(&mut rig, 20, bryn);
    // The release: Ada gives F up.
    rig.variant = "a release by choice".to_owned();
    let out = claim_go(&mut rig, ada, &format!("+claim/release {big} go"), None);
    assert!(
        out.contains(&format!("You release block {big} at Kerwan Floor.")),
        "{out}"
    );
    let _ = fires(&mut rig, 12, ada);
    // A re-stake of the reopened block: Bryn reads F and stakes it.
    scanned_in(&mut rig, KERWAN, (bryn, 1, tb), big);
    rig.variant = "a re-stake of a reopened block".to_owned();
    let out = claim_go(&mut rig, bryn, "+claim/stake go", Some("stake"));
    assert!(out.contains(&format!("You stake block {big} at ")), "{out}");
    let _ = fires(&mut rig, 12, bryn);
    // A leg onto a claimed block (the brief's L5 `start.drive`/`done.drive` variant; the review's
    // tests/T1): Bryn's rig drives to a field of the block he now holds.
    rig.variant = "onto a claimed block".to_owned();
    let mut drove = false;
    for corner in ["NE", "SW", "NW", "SE"] {
        if try_act(&mut rig, bryn, &format!("+tv/1 drive {big}-{corner} go")).is_ok() {
            let _ = finish(&mut rig, tb, bryn);
            drove = true;
            break;
        }
    }
    assert!(drove, "a leg onto block {big}, which Bryn holds");
    // The renewal: Bryn works 50 t of F by real cycles, then renews.
    let worked = work_block(&mut rig, KERWAN, (bryn, 1, tb), big, 50);
    println!("cost_lane: C1 Bryn worked {worked} t of F");
    rig.variant = "past the worked floor".to_owned();
    let out = claim_go(
        &mut rig,
        bryn,
        &format!("+claim/renew {big} go"),
        Some("renew"),
    );
    println!("cost_lane: C1 renew: {out}");
    let _ = fires(&mut rig, 12, bryn);
    // The withdrawn fee: Ada reads M and stakes it, then cancels its fee before the clerk fires.
    scanned_in(&mut rig, KERWAN, (ada, 1, ta), withdrawn);
    rig.variant = "a stake whose fee is withdrawn".to_owned();
    let _ = claim_go(&mut rig, ada, "+claim/stake go", Some("stake"));
    let fee = block_claim(&mut rig.tw, KERWAN, withdrawn)
        .and_then(|claim| claim.fee)
        .expect("the fee");
    let out = run(&mut rig.tw, ada, &format!("@contract/cancel #{fee}"));
    println!("cost_lane: C1 cancel: {out}");
    rig.variant = "withdrawn".to_owned();
    let heard = expecting(&mut rig, &["claim.unpaid"], |rig| fires(rig, 12, ada));
    assert!(
        block_claim(&mut rig.tw, KERWAN, withdrawn).is_some_and(|claim| claim.lapsed_ic.is_some()),
        "the withdrawn fee lapsed the claim: {heard}"
    );
    // The expired fee (SM.10's seam, `claims.rs` C20b): a stake that never lands and an outage
    // past its fee's real hour, so that fee expires unaccepted; Ada's next stake lands and staff
    // point the landed claim's fee at the expired contract before the clerk's visit.
    scanned_in(&mut rig, KERWAN, (ada, 1, ta), never);
    let (_, dropped) = command(&mut rig, ada, "+claim/stake go");
    assert_eq!(dropped.len(), 1, "one order queued, never landed");
    let expired =
        tacsim::port::claims::claim_order_for_tests(rig.tw.world(), ada).expect("the order");
    outage(&mut rig);
    recharge(&mut rig, ta);
    scanned_in(&mut rig, KERWAN, (ada, 1, ta), expired_block);
    rig.variant = "a stake whose fee expired unaccepted".to_owned();
    let _ = claim_go(&mut rig, ada, "+claim/stake go", Some("stake"));
    let mut record = block_record(&mut rig.tw, KERWAN, expired_block).expect("the record");
    let _ = record
        .claim
        .as_mut()
        .expect("Ada's claim")
        .fee
        .replace(expired);
    put_block(&mut rig, record);
    rig.variant = "expired".to_owned();
    let heard = expecting(&mut rig, &["claim.unpaid"], |rig| fires(rig, 30, ada));
    assert!(
        block_claim(&mut rig.tw, KERWAN, expired_block)
            .is_some_and(|claim| claim.lapsed_ic.is_some()),
        "the expired fee lapsed the claim: {heard}"
    );
    // A stake on a papered block: Cass at Occator Rim reads an outer block and stakes it.
    let (cass, _, cass_tvs) = player_on(&mut rig, ("Cass", 3, "Tern"), ("occator", OCCATOR), 1);
    let tc = cass_tvs[0];
    let _ = act_and_finish(
        &mut rig,
        cass,
        tc,
        "+tv/1 launch go",
        "first launch at the site",
    );
    scanned_in(&mut rig, OCCATOR, (cass, 1, tc), papered);
    rig.variant = "on a papered block".to_owned();
    let out = claim_go(&mut rig, cass, "+claim/stake go", Some("stake"));
    println!("cost_lane: C1 papered stake: {out}");
    let _ = fires(&mut rig, 12, cass);
    // The term lapse (SM.10's seam): to Cass's claim's term end, and the clerk's lapse.
    let term_end = block_claim(&mut rig.tw, OCCATOR, papered)
        .map(|claim| claim.term_end_ic)
        .expect("Cass's claim");
    // Nothing paged up to the term's jump; the jump's own pages are the census's (below).
    assert_no_page(&rig, "the tenure census before the term's jump");
    let hulls_out = [ada, bryn, cass]
        .into_iter()
        .filter(|pilot| {
            tacsim::port::live_charter(rig.tw.world(), *pilot).is_some_and(|charter| {
                docked_at(&rig.tw, charter.hull).as_deref() != Some("ceres_port")
            })
        })
        .count();
    rig.variant = "the term's end".to_owned();
    expecting(&mut rig, &["claim.lapse"], |rig| {
        let _ = to_ic(rig, term_end + 60);
        let _ = fires(rig, 30, cass);
    });
    // The unlabelled-claim guard, on the pure core.
    let unlabelled = cohort::kind_of(
        mush_industry::ACTION_CLAIM_TENURE,
        &json!({ "receipt": "tacsim.claim/1/2" }),
        None,
        None,
    );
    assert_eq!(unlabelled, "claim.unlabelled");
    assert!(!cohort::unclassified([(unlabelled.as_str(), "industry_claim_tenure")]).is_empty());
    // **The census's own pages**: the term-lapse seam jumps the clock past a claim's 168 IC-day
    // term, so every chartered hull standing on a site's pad turns dormant, and the recall arm —
    // which recalls a dormant hull only from Ceres — pages each once (doc 18 cut IC13's page).
    // Exactly those, nothing else.
    let own: BTreeMap<&'static str, u64> = [("recall", u64::try_from(hulls_out).unwrap_or(0))]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .collect();
    census_close_paging(&rig, test, &own);
    for (kind, variant) in [
        ("claim.unpaid", "withdrawn"),
        ("claim.unpaid", "expired"),
        ("claim.stake", "on a papered block"),
    ] {
        println!(
            "cost_lane: C1 {kind} {variant} {}",
            variant_of(&rig, kind, variant)
        );
    }
}

// ---------------------------------------------------------------------------
// C1 — the write census, at the port
// ---------------------------------------------------------------------------

/// Occator Rim's yard row.
fn yard(tw: &TestWorld) -> tacsim::port::YardRow {
    port_map(tw, OCCATOR)
        .site
        .and_then(|site| site.yard)
        .expect("Occator Rim's yard is laid up")
}

/// A vendor's storefront vault at Ceres.
fn storefront(tw: &TestWorld, org: &str) -> VaultId {
    VaultId(
        port_map(tw, "ceres_port")
            .vendors
            .get(&cid(org))
            .expect("the vendor's row")
            .vault,
    )
}

/// A Ceres vendor's ask of `good`: its listing id and quantity.
fn ask_of(tw: &TestWorld, org: &str, good: &str) -> Option<(u64, u64)> {
    let market = tw.world().module_state::<mush_market::MarketStore>()?;
    market
        .lot_listing_of(EndpointRef(storefront(tw, org)), &cid(good))
        .and_then(|id| market.listing(id).map(|l| (id.0, l.quantity)))
}

/// The landed count of `kind` so far.
fn landed_count(rig: &Rig, kind: &str) -> u64 {
    rig.tally.kinds.get(kind).map_or(0, |row| row.landed)
}

/// **C1, the port** (doc 20 §12.3; SM.12a, SM.12b; the brief's L5): on the shipped lane, the
/// Exchange's shipped batch (`mill.batch`, 5 writes exact) and its restock (`mill.restock`,
/// SM.12a's 2 exact, at its §11 target of 3); Occator Rim opened, so the site clerk seeds the
/// house's licence item (`site.house_licence`, 3 exact) and its surveys (`dispatch.survey`, 1
/// exact); the house's drones mine and the dispatcher files their plans (`start.plan_file`,
/// attributed house) and delivers the yard hold's ice (`dispatch.deliver`: 6 into a fresh
/// storefront pool, then 5, exact); and the relist variant — the Exchange's `metals` ask sold out
/// through the real verbs, a second batch milled, the clerk's relisting read `List` (`other.list`,
/// 3 writes printed) and never `mill.restock`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12c_every_act_kind_counts_its_writes_at_the_port() {
    let _lock = shared();
    let test = "sm12c_every_act_kind_counts_its_writes_at_the_port";
    let mut rig = stocked(Roster::default());
    // The shipped batch and its restock.
    let exchange = storefront(&rig.tw, "freeport_exchange");
    let listed = ask_of(&rig.tw, "freeport_exchange", "metals").expect("the metals ask");
    seed_pool(&mut rig, exchange, "metal_ore", 20, LotId(9_501));
    rig.variant = "the shipped batch".to_owned();
    expecting(&mut rig, &["mill.batch", "mill.restock"], |rig| {
        for _ in 0..40 {
            let _ = fire(rig);
            if landed_count(rig, "mill.restock") >= 1 {
                break;
            }
        }
    });
    let restocked = ask_of(&rig.tw, "freeport_exchange", "metals").expect("the ask");
    assert_eq!(restocked.0, listed.0, "the same listing, raised");
    assert_eq!(restocked.1, listed.1 + 8, "by the 8 t milled");
    assert_eq!(
        constructed(&rig, "mill.batch"),
        5,
        "the shipped batch (SM.12a 5)"
    );
    assert_eq!(constructed(&rig, "mill.restock"), 2, "SM.12a 2 handed");
    // Occator Rim opened: the house's licence item and its surveys.
    rig.variant = "the site clerk's seeds".to_owned();
    expecting(
        &mut rig,
        &["site.house_licence", "dispatch.survey"],
        |rig| {
            open_site(rig, OCCATOR);
            for _ in 0..200 {
                if yard(&rig.tw).licence.is_some() && landed_count(rig, "dispatch.survey") >= 4 {
                    break;
                }
                let _ = fire(rig);
            }
        },
    );
    assert_eq!(
        constructed(&rig, "site.house_licence"),
        3,
        "SM.12b 3 handed"
    );
    assert_eq!(constructed(&rig, "dispatch.survey"), 1, "SM.12b 1 handed");
    // The drones: the dispatcher's filings and two deliveries.
    expecting(&mut rig, &["dispatch.deliver", "start.plan_file"], |rig| {
        for _ in 0..3_000 {
            let delivered = landed_count(rig, "dispatch.deliver");
            if delivered >= 2 {
                break;
            }
            rig.variant = if delivered == 0 {
                "the house's filing, and the first delivery into a fresh storefront pool"
            } else {
                "a later delivery"
            }
            .to_owned();
            let _ = fire(rig);
        }
    });
    assert_eq!(
        variant_of(
            &rig,
            "dispatch.deliver",
            "the house's filing, and the first delivery into a fresh storefront pool"
        ),
        6,
        "SM.12b 6 into a fresh storefront pool"
    );
    assert_eq!(
        variant_of(&rig, "dispatch.deliver", "a later delivery"),
        5,
        "SM.12b 5"
    );
    // The relist variant. The quote path cannot take the whole ask (a Draw lands in the buyer's
    // hold, 100 t; the ask is 4,008 t), so the listing is let lapse at its real deadline (the
    // brief's alternative): the clock to just before it, a second batch milled there and landed,
    // then past the deadline before the restock can raise the lapsed listing — and the clerk's
    // list step relists the whole pool with its mill counter set (SM.12a's 3-write relisting).
    let (id, _) = ask_of(&rig.tw, "freeport_exchange", "metals").expect("the ask");
    let deadline = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| m.listing(mush_market::ListingId(id)).map(|l| l.expires.at))
        .expect("the ask's deadline");
    println!(
        "cost_lane: C1 the metals ask #{id} lapses at {deadline} (now {})",
        rig.tw.world().now()
    );
    let _ = step_to(&mut rig, deadline - 3_600);
    seed_pool(&mut rig, exchange, "metal_ore", 20, LotId(9_502));
    rig.variant = "a second batch before the lapse".to_owned();
    let batches = landed_count(&rig, "mill.batch");
    for _ in 0..400 {
        let _ = fire(&mut rig);
        if landed_count(&rig, "mill.batch") > batches {
            break;
        }
    }
    assert!(
        landed_count(&rig, "mill.batch") > batches && rig.tw.world().now() < deadline,
        "the second batch landed before the deadline"
    );
    let _ = step_to(&mut rig, deadline + 1);
    rig.variant = "a relisting after a lapse".to_owned();
    // `mill.restock` is expected here too, so a relisting misread as a restock would record this
    // variant under it (the review's tests/T3), and its landed count is read across the window.
    let restocks = landed_count(&rig, "mill.restock");
    expecting(&mut rig, &["other.list", "mill.restock"], |rig| {
        for _ in 0..200 {
            if ask_of(&rig.tw, "freeport_exchange", "metals").is_some_and(|(now, _)| now != id) {
                break;
            }
            let _ = fire(rig);
        }
    });
    let relisted = ask_of(&rig.tw, "freeport_exchange", "metals").expect("relisted");
    println!("cost_lane: C1 the relisting: {relisted:?}");
    let list = rig
        .tally
        .kinds
        .get("other.list")
        .and_then(|row| row.variants.get("a relisting after a lapse").copied());
    println!("cost_lane: C1 other.list (SM.12a's relisting with a counter, 3) {list:?}");
    assert_eq!(
        list,
        Some(3),
        "the relisting is List with its counter write, never a restock"
    );
    assert_eq!(
        rig.tally
            .kinds
            .get("mill.restock")
            .and_then(|row| row.variants.get("a relisting after a lapse")),
        None,
        "never mill.restock"
    );
    assert_eq!(
        landed_count(&rig, "mill.restock"),
        restocks,
        "no restock landed in the relisting's window"
    );
    census_close(&rig, test);
}

// ---------------------------------------------------------------------------
// C4 — the site-ground LRU under eighteen grounds
// ---------------------------------------------------------------------------

/// The lane's six sites, in the lane's site order (`tests/sites.rs`' `SITES`).
const SITES: [&str; 6] = support::sites::SITE_PORTS;

/// A third test key for C4's eighteen grounds (`KEY_1` and `KEY_2` are the suites' own).
const KEY_3: [u8; 32] = *b"rustmush-sm12c-cost-lane-key-003";

/// **C4 — the LRU under eighteen grounds** (doc 20 §11's composite churn; the brief's L8): on a
/// booted keyed world, the six lane sites' frames and specs each captured once, then built under
/// three keys (versions 1, 2, 3) into a sixteen-entry cache — eighteen grounds — round-robin for
/// three rounds, each access a captured-peek miss and then a build and an insert: exactly 54
/// builds, 38 evictions and 16 entries. Each access's capture + revision + build is timed (the
/// warm's own order), and the warm peek (capture + captured hit) and the cold peek (capture +
/// `site_revision` + miss), 30 samples each — decision (a)'s inputs, printed, never asserted.
#[test]
fn sm12c_the_site_ground_lru_under_eighteen_grounds() {
    let _lock = exclusive();
    let (world, _) = mush_testkit::boot_world();
    let tw = boot_over(world, HashMap::new(), &Roster::default());
    let w = tw.world();
    let mut sites = Vec::new();
    for entry in SITES {
        let (spec, location) = spec_of(&tw, entry);
        let frame = site_frame(&location);
        let input = tacsim::surface::ground::capture_site(w, &frame).expect("a capture");
        let body = tacsim::surface::body_geology(w, &spec.body).expect("the body's table");
        sites.push((entry, frame, spec, body, input));
    }
    let keys = [
        (1_u32, geology_keyed::mac(KEY_1)),
        (2, geology_keyed::mac(KEY_2)),
        (3, geology_keyed::mac(KEY_3)),
    ];
    let rounds: u64 = 3;
    let mut cache = tacsim::surface::SiteGroundCache::default();
    let mut access_us = Vec::new();
    for _ in 0..rounds {
        for (version, mac) in &keys {
            for (_, frame, spec, body, _) in &sites {
                let started = Instant::now();
                let input = tacsim::surface::ground::capture_site(w, frame).expect("a capture");
                assert!(
                    cache.peek_captured(&input, spec, *version).is_none(),
                    "a round-robin over eighteen misses every time"
                );
                let revision =
                    tacsim::surface::ground::site_revision(&input, spec).expect("a revision");
                let ground =
                    tacsim::surface::ground::build_site_ground(&input, spec, body, mac, *version)
                        .expect("the site builds");
                let key = tacsim::surface::SiteGroundKey {
                    region: input.frame.region.clone(),
                    revision,
                    key_version: *version,
                };
                let _ = cache.insert_captured(key, ground, input, spec.clone());
                access_us.push(micros(started));
            }
        }
    }
    let counted = cache.counters();
    println!(
        "cost_lane: C4 counters builds {} evictions {} entries {} (rounds {rounds})",
        counted.builds, counted.evictions, counted.entries
    );
    assert_eq!(
        (counted.builds, counted.evictions, counted.entries),
        (18 * rounds, 18 * rounds - 16, 16)
    );
    // The peeks: warm (the last site under the last key is in the cache) and cold (a key
    // version no entry was built under).
    let (_, frame, spec, _, _) = sites.last().expect("a site");
    let (mut warm_us, mut cold_us) = (Vec::new(), Vec::new());
    for _ in 0..30 {
        let started = Instant::now();
        let input = tacsim::surface::ground::capture_site(w, frame).expect("a capture");
        assert!(
            cache.peek_captured(&input, spec, 3).is_some(),
            "a warm peek hits"
        );
        warm_us.push(micros(started));
        let started = Instant::now();
        let input = tacsim::surface::ground::capture_site(w, frame).expect("a capture");
        let revision = tacsim::surface::ground::site_revision(&input, spec).expect("a revision");
        let key = tacsim::surface::SiteGroundKey {
            region: input.frame.region.clone(),
            revision,
            key_version: 9,
        };
        assert!(cache.peek(&key).is_none(), "a cold peek misses");
        cold_us.push(micros(started));
    }
    println!(
        "cost_lane: C4 capture+build p50 {} p99 {} max {} us ({} accesses; build {})",
        cohort::p50(&access_us),
        cohort::p99(&access_us),
        cohort::max(&access_us),
        access_us.len(),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!(
        "cost_lane: C4 warm peek p50 {} p99 {} max {} us; cold peek p50 {} p99 {} max {} us",
        cohort::p50(&warm_us),
        cohort::p99(&warm_us),
        cohort::max(&warm_us),
        cohort::p50(&cold_us),
        cohort::p99(&cold_us),
        cohort::max(&cold_us)
    );
}

// ---------------------------------------------------------------------------
// The cohort's lane (O-21) and C7
// ---------------------------------------------------------------------------

/// The lane's bytes as JSON, edited by `edit`, at `version`, re-signed (`mill.rs`'s).
fn lane_variant(version: u32, edit: impl FnOnce(&mut Value)) -> String {
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = json!(version);
    doc["manifest"]["digest"] = json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

/// **The cohort lane's levers** (O-21, the owner's default: a test-only seed variant sized by the
/// builder, every value printed against the shipped one). Only existing levers move.
#[derive(Clone, Copy, Debug)]
struct Levers {
    /// Every Ceres ore bid's intake (and its drip's cap).
    intake: u64,
    /// Every Ceres ore bid's drip a day.
    per_ic_day: u64,
    /// The Freeport Exchange's capital, major units.
    exchange_capital: u64,
    /// One tranche appended to Occator's raises, major units (at most `MAX_TRANCHE_MAJOR`).
    occator_raise: u64,
    /// Occator's propellant stock row.
    propellant: u64,
    /// The Co-op's provisions stock row.
    provisions: u64,
    /// Both mills' daily rate, tonnes of ore.
    mill_per_day: u64,
    /// Both mills' batch, tonnes of ore (a whole number of 5 : 2 ratios).
    mill_batch: u64,
}

impl Default for Levers {
    /// **The sizing** (the brief's L11.1): thirty Ceres-site hulls each selling about a hold (86
    /// t) a loop of about two and a half IC days is about 1,000 t of ore an IC day across the
    /// lane, about half metal and half ice — so each ore bid drips 600 t a day into an intake of
    /// 1,200 (no hull waits a day on a full intake), the mills mill 700 t a day in batches of 100
    /// (the drip plus the yard's own yield, O-10's bound), the Exchange's capital doubles and
    /// Occator takes one more tranche of 800,000 for its ice, and the chandlery's stock covers 32
    /// hulls bunkering about twelve times.
    fn default() -> Levers {
        Levers {
            intake: 1_200,
            per_ic_day: 600,
            exchange_capital: 8_000_000,
            occator_raise: 800_000,
            propellant: 20_000,
            provisions: 10_000,
            mill_per_day: 700,
            mill_batch: 100,
        }
    }
}

/// **The cohort's lane** (the brief's L11.1): the shipped lane at `LANE_VERSION`, editing only
/// existing levers — the Co-op programme's pool (`hulls`) and names (the eight shipped, then
/// `Cohort 09`, …), the Trust's capital coupled to it (`floor + hulls × stake`), Occator's rig
/// pool (`rigs`), the ore bids' intake and drip, the Exchange's capital and one Occator tranche,
/// the propellant and provisions stock rows, both mills' rate and batch.
fn cohort_lane(hulls: u32, rigs: u32, levers: &Levers) -> String {
    lane_variant(LANE_VERSION, |doc| {
        let def = &mut doc["entries"]["ceres_port"]["def"];
        let shipped: Vec<Value> = def["programme"]["hull_names"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let names: Vec<Value> = (0..hulls)
            .map(|n| {
                shipped
                    .get(usize::try_from(n).unwrap_or(usize::MAX))
                    .cloned()
                    .unwrap_or_else(|| json!(format!("Cohort {:02}", n + 1)))
            })
            .collect();
        def["programme"]["pool"] = json!(hulls);
        def["programme"]["hull_names"] = Value::Array(names);
        let floor = def["programme"]["floor_major"].as_u64().unwrap_or(0);
        let stake = def["programme"]["stake_major"].as_u64().unwrap_or(0);
        def["vehicles"]["pool"] = json!(rigs);
        if let Some(orgs) = def["orgs"].as_array_mut() {
            for org in orgs {
                match org["id"].as_str() {
                    Some("coop_trust") => {
                        org["capital_major"] = json!(floor + u64::from(hulls) * stake);
                    }
                    Some("freeport_exchange") => {
                        org["capital_major"] = json!(levers.exchange_capital);
                    }
                    Some("occator") => {
                        if let Some(raises) = org.get_mut("raises").and_then(Value::as_array_mut) {
                            raises.push(json!(levers.occator_raise));
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(vendors) = def["vendors"].as_array_mut() {
            for vendor in vendors {
                if let Some(bids) = vendor.get_mut("bids").and_then(Value::as_array_mut) {
                    for bid in bids {
                        let ore = bid["good"].as_str().is_some_and(|g| g.ends_with("_ore"));
                        if ore && bid.get("refresh").is_some() {
                            bid["intake"] = json!(levers.intake);
                            bid["refresh"]["per_ic_day"] = json!(levers.per_ic_day);
                            bid["refresh"]["cap"] = json!(levers.intake);
                        }
                    }
                }
                let org = vendor["org"].as_str().map(str::to_owned);
                if let Some(stock) = vendor.get_mut("stock").and_then(Value::as_array_mut) {
                    for row in stock {
                        match (org.as_deref(), row["good"].as_str()) {
                            (Some("occator"), Some("propellant")) => {
                                row["qty"] = json!(levers.propellant);
                            }
                            (Some("coop"), Some("provisions")) => {
                                row["qty"] = json!(levers.provisions);
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(mills) = vendor.get_mut("mill").and_then(Value::as_array_mut) {
                    for mill in mills {
                        mill["per_ic_day_t"] = json!(levers.mill_per_day);
                        mill["batch_t"] = json!(levers.mill_batch);
                    }
                }
            }
        }
    })
}

/// The shipped lane's digest (`tests/lane.rs`' `LANE_DIGEST`, lane v15 since doc 20 cut SM.12b).
const SHIPPED_LANE_DIGEST: &str =
    "4969afa44fe21918117a358bbab76fce4b64c9c692b17cdbca500a73c5b6d01d";

/// **C7 — the cohort lane is the shipped lane but its levers** (O-21; the brief's L9.2): the
/// 32-hull, 64-rig variant parses and verifies, carries `manifest.version == LANE_VERSION`, and
/// differs from `LANE_JSON` only at the lever table's paths plus `manifest.digest` (a structural
/// diff, every differing path listed); the programme pool and names, the Trust's capital, the
/// rig pool, the ore bids, the capital, the stock rows and the mills all moved; `concord.relief`
/// and every other entry unmoved; `LANE_VERSION` 15 and the shipped digest unmoved.
#[test]
fn sm12c_the_cohort_lane_is_the_shipped_lane_but_its_levers() {
    let _lock = shared();
    assert_eq!(LANE_VERSION, 18);
    assert_eq!(
        load_pack(LANE_JSON)
            .expect("the lane")
            .manifest()
            .digest()
            .to_hex(),
        SHIPPED_LANE_DIGEST
    );
    let levers = Levers::default();
    let variant = cohort_lane(32, 64, &levers);
    let pack = load_pack(&variant).expect("the cohort lane verifies");
    assert_eq!(pack.manifest().version(), LANE_VERSION);
    let (a, b): (Value, Value) = (
        serde_json::from_str(LANE_JSON).expect("JSON"),
        serde_json::from_str(&variant).expect("JSON"),
    );
    let diff = cohort::json_diff(&a, &b);
    for path in &diff {
        println!(
            "cost_lane: C7 lever {path}: {} -> {}",
            at(&a, path),
            at(&b, path)
        );
    }
    let strays: Vec<&String> = diff.iter().filter(|path| !cohort::is_lever(path)).collect();
    assert!(
        strays.is_empty(),
        "the variant moves only its levers: {strays:?}"
    );
    // **Exactly the levers, both ways** (the review's conformance/F4: the table's wildcards alone
    // would admit another org's capital, a non-ore bid or another vendor's stock row): every path
    // the variant may move, found in the shipped lane by identity — the Trust's and the Exchange's
    // capital and Occator's raises by org id, each ore bid with a drip by its good, Occator's
    // propellant and the Co-op's provisions rows by vendor and good, every mill row's two fields.
    let want = lever_paths_by_identity(&a);
    let got: BTreeSet<String> = diff.iter().cloned().collect();
    assert_eq!(
        got.symmetric_difference(&want).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "the variant differs at exactly the levers' paths"
    );
    assert!(
        diff.iter().all(|path| !path.contains("luna_port")),
        "concord.relief unmoved"
    );
    assert_eq!(
        b["entries"]["ceres_port"]["def"]["orgs"][2]["capital_major"],
        json!(850_000),
        "the Trust's capital: 50,000 + 32 x 25,000"
    );
}

/// **The exact paths the cohort lane moves, found in the shipped lane `doc` by identity** (C7):
/// the manifest's digest; the programme's pool and its names (an array of another length differs
/// at the array); the rig pool; `coop_trust`'s and `freeport_exchange`'s `capital_major` and
/// `occator`'s `raises` (one tranche appended) by org id; every vendor's ore bid that drips (its
/// `intake`, `refresh/per_ic_day`, `refresh/cap`); Occator's propellant and the Co-op's
/// provisions stock rows' `qty`; every mill row's `per_ic_day_t` and `batch_t`.
fn lever_paths_by_identity(doc: &Value) -> BTreeSet<String> {
    let base = "entries/ceres_port/def";
    let def = &doc["entries"]["ceres_port"]["def"];
    let mut out: BTreeSet<String> = [
        "manifest/digest".to_owned(),
        format!("{base}/programme/pool"),
        format!("{base}/programme/hull_names"),
        format!("{base}/vehicles/pool"),
    ]
    .into_iter()
    .collect();
    for (n, org) in def["orgs"].as_array().into_iter().flatten().enumerate() {
        match org["id"].as_str() {
            Some("coop_trust" | "freeport_exchange") => {
                out.insert(format!("{base}/orgs/{n}/capital_major"));
            }
            Some("occator") => {
                out.insert(format!("{base}/orgs/{n}/raises"));
            }
            _ => {}
        }
    }
    for (v, vendor) in def["vendors"].as_array().into_iter().flatten().enumerate() {
        for (b, bid) in vendor["bids"].as_array().into_iter().flatten().enumerate() {
            let ore = bid["good"].as_str().is_some_and(|g| g.ends_with("_ore"));
            if ore && bid.get("refresh").is_some() {
                for field in ["intake", "refresh/per_ic_day", "refresh/cap"] {
                    out.insert(format!("{base}/vendors/{v}/bids/{b}/{field}"));
                }
            }
        }
        for (s, row) in vendor["stock"].as_array().into_iter().flatten().enumerate() {
            if matches!(
                (vendor["org"].as_str(), row["good"].as_str()),
                (Some("occator"), Some("propellant")) | (Some("coop"), Some("provisions"))
            ) {
                out.insert(format!("{base}/vendors/{v}/stock/{s}/qty"));
            }
        }
        for (m, _) in vendor["mill"].as_array().into_iter().flatten().enumerate() {
            for field in ["per_ic_day_t", "batch_t"] {
                out.insert(format!("{base}/vendors/{v}/mill/{m}/{field}"));
            }
        }
    }
    out
}

/// The JSON value at a `/`-joined path, rendered short.
fn at(doc: &Value, path: &str) -> String {
    let mut here = doc;
    for part in path.split('/') {
        here = match (here, part.parse::<usize>()) {
            (Value::Array(items), Ok(n)) => items.get(n).unwrap_or(&Value::Null),
            (value, _) => value.get(part).unwrap_or(&Value::Null),
        };
    }
    let text = here.to_string();
    if text.len() > 80 {
        format!("{}…", &text[..80])
    } else {
        text
    }
}

// ---------------------------------------------------------------------------
// The ring pin (doc 20a §5.3.10)
// ---------------------------------------------------------------------------

/// **A full hopper ring holds the queued unload and never fails it** (doc 20a §5.3.10's always-run
/// pin; the brief's L9.3; SM.11's held start): a rig recovered to its bay with two goods in its
/// hopper, its two-good queued unload standing, the hopper's ring empty; in the same gap 127
/// marked moves out of the hopper through the inventory's own marked move — the first two stamped
/// a day less a hundred real seconds back, the rest now — so the ring holds 127 and admits the
/// unload's two markers only once the first two age, at the hundredth second. Fires 1–99: no
/// unload start, no landing, no `MarkersFull`, `WHY_MARKERS_QUEUED` told once, at least one hold
/// decision (`markers_held`). Then the queued start goes with no further hold, the unload lands by
/// itself and its finish evicts the two aged entries.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12c_a_full_hopper_ring_holds_the_queued_unload_and_never_fails() {
    let _lock = shared();
    let mut rig = stocked(Roster::default());
    let (ada, row, tvs) = player_on(&mut rig, ("Ada", 1, "Salt Moth"), ("ahuna", AHUNA), 1);
    let tv = tvs[0];
    let _ = act_and_finish(&mut rig, ada, tv, "+tv/1 launch go", "a launch");
    fill(&mut rig, tv, "metal_ore", 3, 8_961);
    fill(&mut rig, tv, "ice_ore", 1, 8_962);
    // The recover's finish lands; its tell (and the queued start after it) not yet.
    let _ = act_go(&mut rig, ada, "+tv/1 recover go");
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("the recover in flight");
    let _ = to_ic(&mut rig, due);
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_some()),
        "the two-good queued unload stands"
    );
    let hop = hopper(&rig.tw, tv);
    let hold = hold_vault(&rig.tw, &row);
    let ring = marker_ring(&mut rig.tw, hop).map_or(0, |ring| ring.len());
    assert_eq!(ring, 0, "the hopper's ring is empty before the fill");
    // 127 marked moves in the same gap: the hopper topped up first, so its load is unchanged.
    fill(&mut rig, tv, "metal_ore", 127, 8_963);
    let t0 = rig.tw.world().now();
    let aged = t0 - mush_inventory::MARKER_RETENTION_SECS + 100;
    aged_markers(&mut rig, (hop, hold), "metal_ore", 2, (aged, "pin-aged"));
    aged_markers(&mut rig, (hop, hold), "metal_ore", 125, (t0, "pin-young"));
    // The seam's 127 t landed in the hold; staff take them back out (a plain debit, no marker),
    // so the hold keeps the room the queued unload's split reads.
    debit_pool(&mut rig, hold, "metal_ore", 127);
    assert_eq!(
        marker_ring(&mut rig.tw, hop).map_or(0, |ring| ring.len()),
        127
    );
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 3, "the load unchanged");
    let ring = marker_ring(&mut rig.tw, hop).expect("the ring");
    assert!(!ring.admits(2, t0 + 99), "young at the 99th second");
    assert!(ring.admits(2, t0 + 100), "aged at the 100th");
    // Fires 1–99: held.
    let mut held = 0_u32;
    let mut heard = String::new();
    let mut last_hold = None;
    for n in 1..=99_u32 {
        let fired = second(&mut rig);
        heard.push_str(&said(&rig.tw, &fired.events, ada));
        heard.push('\n');
        if let Some(stats) = fired.surface
            && stats.markers_held > 0
        {
            held = held.saturating_add(stats.markers_held);
            last_hold = Some(n);
        }
        assert!(
            fired
                .landed
                .iter()
                .all(|(kind, _)| !kind.contains("unload")),
            "fire {n}: no unload start or finish while the ring is full: {:?}",
            fired.landed
        );
    }
    let why = tacsim::port::tv::WHY_MARKERS_QUEUED;
    assert_eq!(heard.matches(why).count(), 1, "told once: {heard}");
    // A `MarkersFull` finish is told by its line and recorded on the rig's `last` (the review's
    // correctness/F6, conformance/F7: the enum's name is never told).
    assert!(
        !heard.contains(tacsim::port::tv::WHY_MARKERS_FULL),
        "{heard}"
    );
    assert_eq!(rig.tally.markers_full, 0, "no MarkersFull finish");
    assert!(held >= 1, "at least one hold decision");
    println!("cost_lane: ring pin: {held} hold decisions in fires 1-99, the last at {last_hold:?}");
    // Then the start goes by itself, the unload lands, and its finish evicts the two aged ones.
    let until = 100
        + u32::try_from(tacsim::surface::DONE_BACKOFF_FIRES).unwrap_or(60)
        + tacsim::surface::UNLOAD_S.div_ceil(24)
        + 20;
    let (mut started, mut landed) = (None, None);
    for n in 100..=until {
        let fired = second(&mut rig);
        if let Some(stats) = fired.surface
            && stats.markers_held > 0
        {
            last_hold = Some(n);
        }
        for (kind, _) in &fired.landed {
            if kind == "start.unload.queued" {
                started = Some(n);
            }
            if kind == "done.unload" {
                landed = Some(n);
            }
        }
        if landed.is_some() {
            break;
        }
    }
    println!(
        "cost_lane: ring pin: the last hold at fire {last_hold:?}, the start at {started:?}, the \
         landing at {landed:?}"
    );
    assert!(
        started.is_some() && landed.is_some(),
        "the queued unload went by itself"
    );
    assert!(
        last_hold.is_some_and(|n| n < 100),
        "no hold after the entries aged"
    );
    assert_eq!(
        marker_ring(&mut rig.tw, hop).map_or(0, |ring| ring.len()),
        127,
        "127 - 2 aged + 2 new"
    );
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, hop, "ice_ore"), 0);
    assert_eq!(
        (rig.tally.markers_full, rig.tally.unload_markers),
        (0, 0),
        "the held queue never finished refused, and its offer was never refused"
    );
    assert_kinds_classified(&rig);
    no_refused_writes(&rig.tw);
}

/// The unload finish's hopper-marker horizon, real seconds (`surface::act`'s crate-private
/// `UNLOAD_MARKER_KEEP_S`, doc 20 cut SM.12c decision (c)): one real hour.
const UNLOAD_KEEP_S: i64 = 3_600;

/// **A hopper ring of hour-old markers still unloads its queue** (doc 20 cut SM.12c, decision
/// (c)'s acceptance; the brief's L12 (c)): the ring pin's rig — a rig recovered to its bay with two
/// goods in its hopper and its two-good queued unload standing — with the hopper's ring filled to
/// its cap of 128 by marked moves kept the unload's hour (`transfer_many_marked_keeping`) and
/// stamped two real hours back. The same walls at the default horizon would hold the start (a day
/// is not past); kept, the ring admits the unload's two markers at once. Every fire: no hold
/// (`markers_held` 0), `WHY_MARKERS_QUEUED` never told; the queued start goes at the tick's first
/// decision (fire 1, no back-off), the unload lands, and its finish's two entries
/// carry the hour (each push evicting its eight aged entries from the front: 128 → 114). A hand
/// unload never consults `admits` and would pass with the gap; the queued start is the one that
/// reads the ring.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12c_a_hopper_ring_of_hour_old_markers_still_unloads_its_queue() {
    let _lock = shared();
    let mut rig = stocked(Roster::default());
    let (ada, row, tvs) = player_on(&mut rig, ("Ada", 1, "Salt Moth"), ("ahuna", AHUNA), 1);
    let tv = tvs[0];
    let _ = act_and_finish(&mut rig, ada, tv, "+tv/1 launch go", "a launch");
    fill(&mut rig, tv, "metal_ore", 3, 8_971);
    fill(&mut rig, tv, "ice_ore", 1, 8_972);
    let _ = act_go(&mut rig, ada, "+tv/1 recover go");
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("the recover in flight");
    let _ = to_ic(&mut rig, due);
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_some()),
        "the two-good queued unload stands"
    );
    let hop = hopper(&rig.tw, tv);
    let hold = hold_vault(&rig.tw, &row);
    assert_eq!(
        marker_ring(&mut rig.tw, hop).map_or(0, |ring| ring.len()),
        0
    );
    // 128 kept marked moves stamped two real hours back: the hopper topped up first, the hold
    // emptied after, so the load and the hold's room are unchanged.
    let cap = u64::try_from(mush_inventory::MAX_MARKER_RING).expect("the cap fits");
    fill(&mut rig, tv, "metal_ore", cap, 8_973);
    let t0 = rig.tw.world().now();
    let stamp = rig.tw.world().world_ic_now();
    for k in 0..cap {
        let id = mush_inventory::derived_marker_id(1, &json!({ "sm12c.kept": "hour", "n": k }))
            .expect("a marker id");
        staff_inventory(&mut rig, |families, txn| {
            families
                .transfer_many_marked_keeping(
                    txn,
                    EndpointRef(hop),
                    EndpointRef(hold),
                    &[(cid("metal_ore"), 1, id.clone())],
                    GOD,
                    &stamp,
                    t0 - 2 * UNLOAD_KEEP_S,
                    UNLOAD_KEEP_S,
                )
                .map(|_| ())
        });
    }
    debit_pool(&mut rig, hold, "metal_ore", cap);
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 3, "the load unchanged");
    let ring = marker_ring(&mut rig.tw, hop).expect("the ring");
    assert_eq!(
        ring.len(),
        mush_inventory::MAX_MARKER_RING,
        "the ring at its cap"
    );
    assert!(ring.admits(2, t0), "hour-old kept entries make room");
    let as_default = mush_inventory::MarkerRing::new(
        ring.entries()
            .iter()
            .map(|entry| mush_inventory::RingEntry {
                keep_s: None,
                ..entry.clone()
            })
            .collect(),
    )
    .expect("the same walls");
    assert!(
        !as_default.admits(2, t0),
        "the same walls at the default horizon would hold the start"
    );
    let until = u32::try_from(tacsim::surface::DONE_BACKOFF_FIRES).unwrap_or(60)
        + tacsim::surface::UNLOAD_S.div_ceil(24)
        + 40;
    let (mut started, mut landed) = (None, None);
    let mut heard = String::new();
    for n in 1..=until {
        let fired = second(&mut rig);
        heard.push_str(&said(&rig.tw, &fired.events, ada));
        heard.push('\n');
        if let Some(stats) = fired.surface {
            assert_eq!(stats.markers_held, 0, "fire {n}: no hold on a kept ring");
        }
        for (kind, _) in &fired.landed {
            if kind == "start.unload.queued" {
                started = Some(n);
            }
            if kind == "done.unload" {
                landed = Some(n);
            }
        }
        if landed.is_some() {
            break;
        }
    }
    println!("cost_lane: kept ring: the start at {started:?}, the landing at {landed:?}");
    assert!(
        started.is_some() && landed.is_some(),
        "the queued unload went by itself"
    );
    // **No hold at all**: the queued start goes at the tick's first decision, fire 1 (a held queue
    // backs off `DONE_BACKOFF_FIRES` before its next decision, as the ring pin's start at fire 121
    // shows). This replaces the brief's "`+tv/plan show` names no `WAIT_MARKERS`" clause, which a
    // hand-recovered rig cannot fail: `WAIT_MARKERS` is only `PlanWait::Markers`' text, a live
    // plan's wait (the review's correctness/F6) — the hold itself is `markers_hold`'s one
    // decision for a plan's queue and a hand queue alike, read here as `markers_held` and as the
    // start's fire.
    assert_eq!(started, Some(1), "the start at the first decision");
    assert!(
        !heard.contains(tacsim::port::tv::WHY_MARKERS_QUEUED),
        "never told the log is full: {heard}"
    );
    assert!(
        !heard.contains(tacsim::port::tv::WHY_MARKERS_FULL),
        "{heard}"
    );
    assert_eq!(
        (rig.tally.markers_full, rig.tally.unload_markers),
        (0, 0),
        "no MarkersFull finish and no unload_markers refusal"
    );
    let after = marker_ring(&mut rig.tw, hop).expect("the ring");
    assert_eq!(
        after.len(),
        mush_inventory::MAX_MARKER_RING + 2 - 2 * mush_inventory::MAX_RING_EVICTIONS_PER_PUSH,
        "two pushes, each evicting its eight"
    );
    let tail: Vec<Option<i64>> = after
        .entries()
        .iter()
        .rev()
        .take(2)
        .map(|e| e.keep_s)
        .collect();
    assert_eq!(
        tail,
        vec![Some(UNLOAD_KEEP_S); 2],
        "the finish keeps its markers the hour"
    );
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, hop, "ice_ore"), 0);
    assert_kinds_classified(&rig);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// The cohort: pilots playing the loop between one-second fires (C2, C3 and the month)
// ---------------------------------------------------------------------------

/// The ore goods a hull may carry home, with the org whose standing bid buys each.
const ORES: [(&str, &str); 4] = [
    ("metal_ore", "freeport_exchange"),
    ("ice_ore", "occator"),
    ("carbon_ore", "freeport_exchange"),
    ("strategic_ore", "freeport_exchange"),
];

/// The Dray-P's ore compartments' mass limit, whole tonnes, from content (`unloading.rs`'s).
fn ore_limit_t(tw: &TestWorld) -> u64 {
    let spec = store(tw)
        .definitions()
        .specs
        .get(&cid(tacsim::definitions::content::DRAY_P))
        .cloned()
        .expect("the Dray-P");
    let logistics = spec.logistics.expect("its logistics");
    logistics
        .stowage
        .values()
        .filter(|compartment| compartment.accepts.contains(&cid("stow.ore")))
        .map(|compartment| {
            compartment
                .limits
                .get(&cid("si.mass_kg"))
                .copied()
                .unwrap_or(0)
        })
        .sum::<u64>()
        / 1_000
}

/// How a pilot plays (the brief's L6 and L11.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Scans by hand, files both rigs' fill plans, queues the captain's await, depart and sell.
    Plan,
    /// Launches, scans, sounds, drives, stakes, mines to the hopper and recovers by hand.
    Hand,
}

/// Where a pilot is in the loop.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    /// At Ceres, off the ship: bunker for the site.
    Bunker,
    /// At Ceres, bunkered: board and depart.
    Depart,
    /// Under way to the site; arrives at this IC instant.
    Outbound,
    /// A plan pilot on the pad: the hand scan.
    PlanScan,
    /// A plan pilot: file both rigs' plans.
    PlanFile,
    /// A plan pilot: the captain's queue filed; waits for the hull back at Ceres, sold.
    PlanQueue,
    /// A hand pilot on the pad: launch rig 1 (and drive to the next field).
    HandLaunch,
    /// A hand pilot: scan.
    HandScan,
    /// A hand pilot: sound.
    HandSound,
    /// A hand pilot: drive to the next pocket of the reading, or recover.
    HandDrive,
    /// A hand pilot: stake (by the rules) and mine where the rig stands.
    HandMine,
    /// A hand pilot: the rig on its way home and its queued unload.
    HandRecover,
    /// A hand pilot: lift for Ceres.
    HandLift,
    /// A hand pilot: under way home.
    Homebound,
    /// A hand pilot at Ceres: sell the hold by hand.
    Sell,
    /// Refused at the epoch sky (the Vesta hulls, O-26): nothing more to do.
    Refused,
    /// Home at Ceres with no Ceres site left that yielded it ore (every site's loop came home
    /// dry): the pilot stops flying empty loops, and the fire it parked is the owner's figure.
    Parked,
}

/// One pilot of a cohort.
#[derive(Clone, Debug)]
struct Pilot {
    /// The pilot's index (its stagger and its mode).
    index: usize,
    who: ObjId,
    row: HullRow,
    rigs: Vec<ObjId>,
    /// The site's port entry and venue word.
    site: &'static str,
    venue: &'static str,
    mode: Mode,
    /// The plan pilot's family (`metal_ore` / `ice_ore`).
    family: &'static str,
    phase: Phase,
    /// What the pilot heard since its last act (the sounding's lines are parsed from it).
    heard: String,
    /// The hand pilot's targets, from its own sounding.
    targets: Vec<String>,
    /// The hand pilot's live claims, oldest first.
    claims: Vec<char>,
    /// Loops completed (each a sale at Ceres).
    loops: u32,
    /// The IC instant the phase began.
    since_ic: i64,
    /// The real second a pilot may first act (the stagger).
    start_at: i64,
    /// The block of the hand pilot's last loud sounding (the month's contest reads it).
    loud_block: Option<char>,
    /// The month's renewer: its rounds stay on its first claim's four fields.
    stay_on_claim: bool,
    /// Whether a hand sounding was filed whose lines have not been read yet.
    sound_pending: bool,
    /// The rounds of hand mining on this pad visit.
    rounds: u32,
    /// The first fire index at which the pilot may act again (a short back-off after a refusal).
    retry_at: u64,
    /// The bunker's purchase rounds since it last read settled (each waits on the market's sweep
    /// across measured one-second fires, never a fire of its own).
    bunker_rounds: u32,
    /// Whether a plan pilot filed a plan on this loop (its rigs' plan tallies are this loop's).
    filed: bool,
    /// Whether a plan pilot's filing on this loop found the site yields it nothing (no open block
    /// read, or every plan refused for the ground rather than the hold).
    dry: bool,
    /// The Ceres sites this pilot's own loops came home dry from (it looks elsewhere).
    dry_sites: BTreeSet<&'static str>,
    /// **The families a plan pilot's loops came home dry of, per site** (the owner's ask of
    /// 2026-10-26, after the investigation of the month's parked pilots): a site is dry for the
    /// pilot only once both ore families are, so a metal pilot whose metal is worked out mines the
    /// ice still there rather than leaving the site.
    dry_families: BTreeSet<(&'static str, &'static str)>,
    /// The family the pilot's plans were filed in on this loop (`None` before a filing).
    filed_family: Option<&'static str>,
    /// **The IC instant the pilot last made progress** (the brief's "stuck": a step or a queue
    /// that has not advanced for an IC day, waits included): a phase change, or in the plan's
    /// queue its rigs' plan cursors, cycles or trips or its hull's sailing-plan revision moving
    /// (read every [`PROGRESS_EVERY`] fires). Phase age alone flagged a plan still mining a day
    /// on contended, leaner ground (the 2026-10-26 month, after the dry-rule fix).
    progress_ic: i64,
    /// What the progress clock last read: the rigs' cursors, cycles and trips summed, and the
    /// sailing plan's revision.
    progress_sig: (u64, u64),
}

/// How often, in fires, the month reads a queued pilot's progress.
const PROGRESS_EVERY: u64 = 60;

/// What the cohort counted.
#[derive(Clone, Debug, Default)]
struct CohortTally {
    /// Skips by kind (`stake.yours`, `stake.cap`, …).
    skips: BTreeMap<String, u64>,
    /// Commands timed by verb: `(verb, µs)`.
    timed: BTreeMap<String, Vec<u64>>,
    /// The Vesta refusals' texts.
    vesta: Vec<String>,
    /// Tonnes sold per good.
    sold: BTreeMap<String, u64>,
    /// The longest phase per pilot, IC seconds, and the phase.
    longest: BTreeMap<usize, (i64, String)>,
    /// Bunker shortfalls covered by staff when the market could not (never expected).
    staff_bunkers: u64,
    /// Commands the engine's submit queue answered with back-pressure ("Too many actions are
    /// committing right now", "Your earlier action is still committing").
    back_pressure: u64,
}

/// The engine's back-pressure lines (`mush_core::Engine`'s submit queue: full, or the actor's
/// earlier submission still committing).
const BACK_PRESSURE: [&str; 2] = [
    "Too many actions are committing right now",
    "Your earlier action is still committing",
];

/// The cohort: its pilots and its tally.
#[derive(Clone, Debug, Default)]
struct Cohort {
    pilots: Vec<Pilot>,
    tally: CohortTally,
    /// The fire index (`second` calls since the span began).
    fire: u64,
    /// **The Ceres sites a loop came home dry from**, with the fire it was first seen (the
    /// review's correctness/F5): a pilot whose loop mined nothing moves to the next site it has
    /// not itself found dry (`Pilot::dry_sites`), or parks when none is left — the script never
    /// flies empty loops to a worked-out site. The owner's figure: when each site first ran dry
    /// for a pilot at this cohort.
    worked_out: BTreeMap<&'static str, u64>,
    /// The pilots parked, with the fire each parked at.
    parked: BTreeMap<usize, u64>,
}

/// A command as `who`, timed under `verb`: the reply and what it queued, landed.
fn timed_command(
    rig: &mut Rig,
    tally: &mut CohortTally,
    (who, verb): (ObjId, &str),
    line: &str,
) -> (String, usize) {
    let started = Instant::now();
    let out = run(&mut rig.tw, who, line);
    tally
        .timed
        .entry(verb.to_owned())
        .or_default()
        .push(micros(started));
    if BACK_PRESSURE.iter().any(|line| out.contains(line)) {
        tally.back_pressure = tally.back_pressure.saturating_add(1);
    }
    let queued = drain(rig);
    let n = queued.iter().filter(|submit| submit.actor == who).count();
    let label = if line.starts_with("+claim/stake") {
        Some("stake")
    } else if line.starts_with("+claim/renew") {
        Some("renew")
    } else if line.starts_with("+claim/contest") {
        Some("contest")
    } else {
        None
    };
    for submit in &queued {
        let label = if submit.actor == who { label } else { None };
        let _ = land_measured(rig, submit, label);
    }
    let _ = count_and_fold(rig);
    (out, n)
}

/// Whether `tv` has nothing in flight: no act, no run, no queued unload, no live plan.
fn rig_idle(tw: &mut TestWorld, tv: ObjId) -> bool {
    !busy(tw, tv)
}

/// Whether `tv` is in its bay (no excursion open).
fn in_bay(tw: &mut TestWorld, tv: ObjId) -> bool {
    record_of(tw, tv).is_none_or(|record| record.excursion.is_none())
}

/// The port entry `hull` is docked at, if any.
fn docked_at(tw: &TestWorld, hull: ObjId) -> Option<String> {
    match tacsim::port::ship::berth_of(tw.world(), hull) {
        tacsim::port::ship::Berth::Docked { map, .. } => Some(map.entry.to_string()),
        tacsim::port::ship::Berth::UnderWay | tacsim::port::ship::Berth::Elsewhere => None,
    }
}

/// The ore tonnes in a hold.
fn hold_ore(tw: &mut TestWorld, hold: VaultId) -> u64 {
    ORES.iter().map(|(good, _)| pool(tw, hold, good)).sum()
}

/// Whether the hull's captain's queue has run out (every order done or skipped), or there is none.
/// **The reason a hull's sailing plan stands stopped** (`PlanState::Held` with a stopped hold,
/// `retry: false`): it waits for `+ship/order resume`, as a player's would. A waiting hold
/// (`retry: true`) clears by itself and is not one.
fn queue_held(tw: &TestWorld, hull: ObjId) -> Option<String> {
    match store(tw).orders_state(hull) {
        tacsim::port::orders::OrdersState::Present(orders)
            if orders.state == tacsim::port::orders::PlanState::Held =>
        {
            orders
                .hold
                .as_ref()
                .filter(|hold| !hold.retry)
                .map(|hold| hold.reason.clone())
        }
        _ => None,
    }
}

fn queue_done(tw: &TestWorld, hull: ObjId) -> bool {
    match store(tw).orders_state(hull) {
        tacsim::port::orders::OrdersState::Present(orders) => orders.orders.iter().all(|o| {
            matches!(
                o.state,
                tacsim::port::orders::OrderState::Done | tacsim::port::orders::OrderState::Skipped
            )
        }),
        tacsim::port::orders::OrdersState::Absent
        | tacsim::port::orders::OrdersState::Unreadable => true,
    }
}

/// `Burn B t (tank T t; …)` → `(B, T)`; `Stores C crew-days (locker L; …)` → `(C, L)`
/// (`sm_story.rs`'s).
fn plan_numbers(plan: &str, head: &str, open: &str) -> Option<(u64, u64)> {
    let line = plan.lines().find(|l| l.starts_with(head))?;
    let first = line
        .strip_prefix(head)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let second = line
        .split(open)
        .nth(1)?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((first, second))
}

/// `Leg home from {port}: burn {b} t, {d} crew-days at …` → `(b, d)` (`sm_story.rs`'s).
fn leg_home(plan: &str) -> Option<(u64, u64)> {
    let line = plan.lines().find(|l| l.starts_with("Leg home from "))?;
    let rest = line.split(": burn ").nth(1)?;
    let burn = rest.split_whitespace().next()?.parse().ok()?;
    let days = rest
        .split(", ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((burn, days))
}

/// The crew-days the cohort bunkers past the planner's figures (the pad's days and the laden leg
/// home, `sm_story.rs`'s measured residual, with a plan's own hours on the pad).
const COHORT_EXTRA_CREW_DAYS: u64 = 4;
/// The reaction mass past the planner's figures, tonnes.
const COHORT_EXTRA_BURN_T: u64 = 1;

/// What one round of the bunker did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bunkered {
    /// The tank and the locker read enough for the round trip: depart.
    Ready,
    /// A shortfall was bought (the Draws landed): read again once the market's sweep has seen
    /// their receipts, [`BUNKER_SETTLE_FIRES`] measured fires on.
    Bought,
}

/// The measured one-second fires a pilot waits after a purchase round before it reads the plan
/// again (the three six-second setup fires `sm_story.rs` waits, in production's cadence): the
/// market's sweep sees the Draws' receipts in them. **Never a fire of the bunker's own** (the
/// review's correctness/F1: a mid-span six-second fire is unmeasured and moves the IC clock).
const BUNKER_SETTLE_FIRES: u64 = 18;
/// The purchase rounds a bunker may take before it is reported unsettled.
const BUNKER_ROUNDS: u32 = 4;

/// **One round of the bunker for `venue` at the market** (`sm_story.rs`'s step 3):
/// `+ship/plan <venue>`'s burn and stores with the leg home and the margins; when they fall short,
/// the shortfall bought at the chandlery (`+market/quote` then `+market/accept`, the Draw landed)
/// and [`Bunkered::Bought`] — the caller waits [`BUNKER_SETTLE_FIRES`] of the cohort's own
/// measured fires and reads again. `Err` with the planner's reply when the plan does not read (a
/// Vesta hull at the epoch sky) or the market's when a purchase is refused.
fn bunker(
    rig: &mut Rig,
    tally: &mut CohortTally,
    who: ObjId,
    venue: &str,
) -> Result<Bunkered, String> {
    let chandlery = station_room(&rig.tw, "ceres_port", "room.c08_chandlery");
    teleport(&mut rig.tw, who, chandlery);
    let started = Instant::now();
    let plan = run(&mut rig.tw, who, &format!("+ship/plan {venue}"));
    tally
        .timed
        .entry("ship/plan".to_owned())
        .or_default()
        .push(micros(started));
    let (Some((burn, tank)), Some((days, locker))) = (
        plan_numbers(&plan, "Burn ", "(tank "),
        plan_numbers(&plan, "Stores ", "(locker "),
    ) else {
        return Err(plan);
    };
    let (home_burn, home_days) = leg_home(&plan).unwrap_or((burn, days));
    let want_tank = burn + home_burn + COHORT_EXTRA_BURN_T;
    let want_locker = days + home_days + COHORT_EXTRA_CREW_DAYS;
    if tank >= want_tank && locker >= want_locker {
        return Ok(Bunkered::Ready);
    }
    for (org, good, short) in [
        ("occator", "propellant", want_tank.saturating_sub(tank)),
        ("coop", "provisions", want_locker.saturating_sub(locker)),
    ] {
        if short == 0 {
            continue;
        }
        let Some((listing, _)) = ask_of(&rig.tw, org, good) else {
            return Err(format!("no {good} ask"));
        };
        let quoted = run(
            &mut rig.tw,
            who,
            &format!("+market/quote {listing}={short}"),
        );
        let quote = rig
            .tw
            .world()
            .module_state::<mush_market::MarketStore>()
            .and_then(|m| {
                m.quotes_of_buyer(&mush_org::PrincipalId::Object(who))
                    .map(|q| q.id)
                    .max()
            });
        let Some(quote) = quote.filter(|_| quoted.contains("Quote #")) else {
            return Err(quoted);
        };
        let (accepted, _) = timed_command(
            rig,
            tally,
            (who, "market/accept"),
            &format!("+market/accept {quote}"),
        );
        if !accepted.contains("Agreed.") {
            return Err(accepted);
        }
    }
    Ok(Bunkered::Bought)
}

/// Sell every ore in `hold` by hand at the Exchange Hall (`+market/sell <bid>=all`).
fn sell_hold(rig: &mut Rig, tally: &mut CohortTally, who: ObjId, hold: VaultId) {
    let hall = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, who, hall);
    for (good, org) in ORES {
        let qty = pool(&mut rig.tw, hold, good);
        if qty == 0 {
            continue;
        }
        let Some(bid) = port_map(&rig.tw, "ceres_port")
            .vendors
            .get(&cid(org))
            .and_then(|row| row.bids.get(&cid(good)))
            .map(|bid| bid.listing)
        else {
            continue;
        };
        let (out, _) = timed_command(
            rig,
            tally,
            (who, "market/sell"),
            &format!("+market/sell {bid}=all"),
        );
        if out.contains("Sold ") {
            let left = pool(&mut rig.tw, hold, good);
            *tally.sold.entry(good.to_owned()).or_insert(0) += qty.saturating_sub(left);
        }
    }
}

/// Board at Ceres's gallery (unless already aboard).
fn board_if_ashore(rig: &mut Rig, who: ObjId, hull: ObjId) {
    let at = rig.tw.world().get(who).and_then(|o| o.location);
    let aboard = at.is_some_and(|room| {
        tacsim::port::ship::hull_of_room(rig.tw.world(), room)
            .is_some_and(|(view, _)| view.row.hull == hull.0)
    });
    if !aboard {
        board(rig, who);
    }
}

/// Leave the ship at Ceres (`+ship/leave` from the Airlock), if aboard.
fn leave_ship(rig: &mut Rig, who: ObjId) {
    let _ = run(&mut rig.tw, who, "+ship/leave");
}

/// **Whether a plan pilot's loop came home dry**: its filing found the site yields it nothing
/// (`Pilot::dry`: no open block read, or every plan refused for the ground), or its rigs' plans
/// (this loop's, `Pilot::filed`) moved no tonne into a hopper (`PlanTally::tonnes`) **and every
/// one of them ended worked out** — the ground's own verdict. A loop that filed nothing because of
/// its hold is not the site's, and neither is a plan a right or the world ended (`Claimed`,
/// `Licence`, `Lease`, `Dark`, `Stranded`, …: the investigation of 2026-10-26 found a plan ended
/// `Claimed` by another pilot's stake counted dry with 65 fair metal pockets still unreached).
fn plan_loop_dry(rig: &mut Rig, p: &Pilot) -> bool {
    if !p.filed {
        return p.dry;
    }
    let plans: Vec<tacsim::surface::plan::FillPlan> = p
        .rigs
        .iter()
        .filter_map(|tv| record_of(&mut rig.tw, *tv).and_then(|record| record.plan))
        .collect();
    let tonnes: u64 = plans.iter().map(|plan| u64::from(plan.tally.tonnes)).sum();
    tonnes == 0
        && plans.iter().all(|plan| {
            matches!(
                plan.state,
                tacsim::surface::plan::FillState::Ended {
                    why: tacsim::surface::plan::PlanEnd::WorkedOut,
                    ..
                }
            )
        })
}

impl Cohort {
    /// Move `p`'s phase on, stamping the IC instant.
    fn enter(pilot: &mut Pilot, phase: Phase, now_ic: i64) {
        pilot.phase = phase;
        pilot.since_ic = now_ic;
        pilot.progress_ic = now_ic;
        pilot.heard.clear();
    }

    /// **A loop's end at Ceres** (the review's correctness/F5): a loop that came home `dry` adds
    /// its site to the pilot's own dry sites (and the site's first dry fire to the cohort's
    /// figure) and moves the pilot to the next Ceres site in the lane's order it has not found
    /// dry, its claims there left behind — a player looks elsewhere, never flies empty loops to a
    /// site that yields it nothing; with none left it parks. Otherwise, or after the move, the
    /// pilot bunkers for its (new) site.
    fn after_loop(&mut self, p: &mut Pilot, dry: bool, now_ic: i64) {
        let filed_family = p.filed_family.take();
        p.filed = false;
        p.dry = false;
        // A plan pilot's dry loop marks the family it filed in dry at the site (or both, when
        // nothing filed); the site is dry only once both families are (the owner's ask of
        // 2026-10-26): the pilot stays and tries the other family next loop.
        let dry = if dry && p.mode == Mode::Plan {
            match filed_family {
                Some(family) => {
                    let _ = p.dry_families.insert((p.site, family));
                }
                None => {
                    let _ = p.dry_families.insert((p.site, "metal_ore"));
                    let _ = p.dry_families.insert((p.site, "ice_ore"));
                }
            }
            let both = ["metal_ore", "ice_ore"]
                .iter()
                .all(|family| p.dry_families.contains(&(p.site, *family)));
            if !both {
                println!(
                    "cost_lane: pilot {} came home dry of {} at {}; it tries the other family \
                     there (fire {})",
                    p.index,
                    filed_family.unwrap_or("?"),
                    p.site,
                    self.fire
                );
            }
            both
        } else {
            dry
        };
        if dry && let Some(at) = CERES_SITES.iter().position(|(site, _)| *site == p.site) {
            let _ = self.worked_out.entry(p.site).or_insert(self.fire);
            let _ = p.dry_sites.insert(p.site);
            let next = (1..CERES_SITES.len())
                .map(|k| CERES_SITES[(at + k) % CERES_SITES.len()])
                .find(|(site, _)| !p.dry_sites.contains(site));
            match next {
                Some((site, venue)) => {
                    println!(
                        "cost_lane: pilot {} came home dry from {}; on to {site} (fire {})",
                        p.index, p.site, self.fire
                    );
                    p.site = site;
                    p.venue = venue;
                    p.claims.clear();
                    p.stay_on_claim = false;
                }
                None => {
                    println!(
                        "cost_lane: pilot {} parks: every Ceres site came home dry (fire {})",
                        p.index, self.fire
                    );
                    let _ = self.parked.insert(p.index, self.fire);
                    Self::enter(p, Phase::Parked, now_ic);
                    return;
                }
            }
        }
        Self::enter(p, Phase::Bunker, now_ic);
    }

    /// **One second of play** (the brief's L6, L11.3): what each pilot heard this fire, then at
    /// most one decision per pilot — every command between fires, never inside a fire's clock.
    fn step(&mut self, rig: &mut Rig, fired: &Fired) {
        self.fire = self.fire.saturating_add(1);
        let now = rig.tw.world().now();
        let now_ic = ic_now(&rig.tw);
        // The progress clock of every queued plan pilot (`Pilot::progress_ic`).
        if self.fire.is_multiple_of(PROGRESS_EVERY) {
            for p in self
                .pilots
                .iter_mut()
                .filter(|p| p.phase == Phase::PlanQueue)
            {
                let mut moved = 0_u64;
                for tv in &p.rigs {
                    if let Some(plan) = record_of(&mut rig.tw, *tv).and_then(|record| record.plan) {
                        moved = moved
                            .saturating_add(u64::from(plan.cursor))
                            .saturating_add(u64::from(plan.tally.cycles))
                            .saturating_add(u64::from(plan.tally.trips));
                    }
                }
                let rev = match store(&rig.tw).orders_state(ObjId(p.row.hull)) {
                    tacsim::port::orders::OrdersState::Present(orders) => orders.rev,
                    _ => 0,
                };
                if (moved, rev) != p.progress_sig {
                    p.progress_sig = (moved, rev);
                    p.progress_ic = now_ic;
                }
            }
        }
        for n in 0..self.pilots.len() {
            let who = self.pilots[n].who;
            let text = said(&rig.tw, &fired.events, who);
            if !text.is_empty() {
                self.pilots[n].heard.push_str(&text);
                self.pilots[n].heard.push('\n');
            }
            if now < self.pilots[n].start_at || self.fire < self.pilots[n].retry_at {
                continue;
            }
            let span = now_ic - self.pilots[n].since_ic;
            let entry = self.tally.longest.entry(n).or_insert((0, String::new()));
            if span > entry.0 {
                *entry = (span, format!("{:?}", self.pilots[n].phase));
            }
            let mut pilot = self.pilots[n].clone();
            self.decide(rig, &mut pilot, now_ic);
            self.pilots[n] = pilot;
        }
    }

    /// One pilot's one decision.
    #[allow(clippy::too_many_lines)]
    fn decide(&mut self, rig: &mut Rig, p: &mut Pilot, now_ic: i64) {
        let hull = ObjId(p.row.hull);
        let hold = hold_vault(&rig.tw, &p.row);
        let tv1 = p.rigs[0];
        let tally = &mut self.tally;
        match p.phase.clone() {
            Phase::Refused | Phase::Parked => {}
            Phase::Bunker => match bunker(rig, tally, p.who, p.venue) {
                Ok(Bunkered::Ready) => {
                    p.bunker_rounds = 0;
                    Self::enter(p, Phase::Depart, now_ic);
                }
                Ok(Bunkered::Bought) => {
                    p.bunker_rounds = p.bunker_rounds.saturating_add(1);
                    if p.bunker_rounds >= BUNKER_ROUNDS {
                        println!(
                            "cost_lane: pilot {} bunker: it did not settle in {BUNKER_ROUNDS} \
                             rounds",
                            p.index
                        );
                        p.bunker_rounds = 0;
                        p.retry_at = self.fire + 60;
                    } else {
                        p.retry_at = self.fire + BUNKER_SETTLE_FIRES;
                    }
                }
                Err(why) if p.site == "rheasilvia_port" => {
                    tally.vesta.push(why);
                    Self::enter(p, Phase::Depart, now_ic);
                }
                Err(why) => {
                    let (posted, held) = funds(&rig.tw, p.who);
                    println!(
                        "cost_lane: pilot {} bunker (posted {posted}, held {held}): {why}",
                        p.index
                    );
                    p.bunker_rounds = 0;
                    p.retry_at = self.fire + 60;
                }
            },
            Phase::Depart => {
                board_if_ashore(rig, p.who, hull);
                let line = format!("+ship/undock {}", p.venue);
                let (out, n) = timed_command(rig, tally, (p.who, "ship/undock"), &line);
                if n > 0 {
                    Self::enter(p, Phase::Outbound, now_ic);
                } else if p.site == "rheasilvia_port" {
                    tally.vesta.push(out);
                    if tally.vesta.len() >= 4 {
                        Self::enter(p, Phase::Refused, now_ic);
                    } else {
                        p.retry_at = self.fire + 600;
                    }
                } else {
                    if !out.contains("have not come aboard") {
                        println!("cost_lane: pilot {} undock: {out}", p.index);
                    }
                    p.retry_at = self.fire + 6;
                }
            }
            Phase::Outbound => {
                if docked_at(&rig.tw, hull).as_deref() == Some(p.site) {
                    p.rounds = 0;
                    let next = match p.mode {
                        Mode::Plan => Phase::PlanScan,
                        Mode::Hand => Phase::HandLaunch,
                    };
                    Self::enter(p, next, now_ic);
                }
            }
            Phase::PlanScan => {
                if !rig_idle(&mut rig.tw, tv1) {
                    return;
                }
                // From the bay the rig drives out to an outer block's middle first (the scan there
                // reads open ground: Occator holds the four blocks around every pad's ramp at
                // Occator Rim), one block a loop.
                if in_bay(&mut rig.tw, tv1) && p.heard.is_empty() {
                    let fields = ["B", "C", "E", "H", "I", "L", "N", "O"];
                    let at =
                        usize::try_from(p.loops).unwrap_or(0).wrapping_add(p.index) % fields.len();
                    let line = format!("+tv/1 drive {} go", fields[at]);
                    let (_, n) = timed_command(rig, tally, (p.who, "tv/drive"), &line);
                    if n == 0 {
                        p.retry_at = self.fire + 6;
                    }
                    return;
                }
                if !p.heard.is_empty() && p.heard.contains(" scanned ") {
                    Self::enter(p, Phase::PlanFile, now_ic);
                    return;
                }
                let (out, n) = timed_command(rig, tally, (p.who, "tv/scan"), "+tv/1 scan go");
                if n == 0 {
                    if out.contains("nothing new") {
                        // Scanned from this very cell before: the chart already reads it.
                        Self::enter(p, Phase::PlanFile, now_ic);
                        return;
                    }
                    if !out.contains("still being mapped") {
                        println!("cost_lane: pilot {} scan: {out}", p.index);
                    }
                    p.retry_at = self.fire + 6;
                }
            }
            Phase::PlanFile => {
                if !p.rigs.iter().all(|tv| rig_idle(&mut rig.tw, *tv)) {
                    return;
                }
                let letters = chart_of(&mut rig.tw, p.who, p.site)
                    .map(|chart| {
                        let mut letters: Vec<char> = chart
                            .fields
                            .keys()
                            .filter_map(|index| {
                                let (fx, fy) = tacsim::surface::address::field_of_index(*index)?;
                                tacsim::surface::address::letter_of_block(fx / 2, fy / 2)
                            })
                            .collect();
                        letters.sort_unstable();
                        letters.dedup();
                        letters
                    })
                    .unwrap_or_default();
                if letters.is_empty() {
                    Self::enter(p, Phase::PlanScan, now_ic);
                    return;
                }
                // Only blocks the pilot holds or that read open (the public tenure its own
                // `site.map` draws): a block someone else holds would refuse the whole plan.
                let letters: Vec<char> = letters
                    .into_iter()
                    .filter(|letter| {
                        block_claim(&mut rig.tw, p.site, *letter).is_none_or(|claim| {
                            claim.lapsed_ic.is_some()
                                || claim.holder == mush_org::PrincipalId::Object(p.who)
                        })
                    })
                    .collect();
                if letters.is_empty() {
                    *tally
                        .skips
                        .entry("plan.no_open_block".to_owned())
                        .or_insert(0) += 1;
                    // Every block it read is held by someone else: the site yields this pilot
                    // nothing.
                    p.dry = true;
                    Self::enter(p, Phase::PlanQueue, now_ic);
                    go_home(rig, tally, p);
                    return;
                }
                let list: Vec<String> = letters.iter().map(char::to_string).collect();
                let other = if p.family == "metal_ore" {
                    "ice_ore"
                } else {
                    "metal_ore"
                };
                let mut filed = 0;
                let mut hold_bound = false;
                let families: Vec<&'static str> = [p.family, other]
                    .into_iter()
                    .filter(|family| !p.dry_families.contains(&(p.site, *family)))
                    .collect();
                for (slot, _) in p.rigs.iter().enumerate() {
                    for family in families.iter().copied() {
                        let line = format!(
                            "+tv/{} plan {} family={family} go",
                            slot + 1,
                            list.join(",")
                        );
                        let (out, n) = timed_command(rig, tally, (p.who, "tv/plan"), &line);
                        if n > 0 {
                            filed += 1;
                            p.filed_family.get_or_insert(family);
                            break;
                        }
                        hold_bound |= out.contains("hold");
                        *tally
                            .skips
                            .entry(format!("plan.refused.{}", refusal_word(&out)))
                            .or_insert(0) += 1;
                    }
                }
                if filed == 0 {
                    // No plan fits (a full hold, or nothing fair): go home and sell. Only
                    // "nothing fair" is the site's: a refusal the hold bounds is the hull's.
                    p.dry = !hold_bound;
                    Self::enter(p, Phase::PlanQueue, now_ic);
                    go_home(rig, tally, p);
                    return;
                }
                p.filed = true;
                for line in [
                    "+ship/order await tv all".to_owned(),
                    "+ship/order depart ceres".to_owned(),
                    format!("+ship/order sell {} all", p.family),
                    format!("+ship/order sell {other} all"),
                ] {
                    let _ = timed_command(rig, tally, (p.who, "ship/order"), &line);
                }
                Self::enter(p, Phase::PlanQueue, now_ic);
            }
            Phase::PlanQueue => {
                // A stopped hold (a sale the counter refused, a departure that failed): the pilot
                // resumes it a minute later, as a player would, and the month counts it by its
                // reason (`queue.held.<words>`). Found 2026-10-26: at 30 TVs Occator held 30 live
                // rig leases against the economy's per-party cap, then 32 (O-19c), so a third ice
                // sale settling at once was refused "contract exceeds a cap" and the plan held. The
                // owner raised the cap to 100 the same day.
                if let Some(reason) = queue_held(&rig.tw, hull) {
                    *tally
                        .skips
                        .entry(format!("queue.held.{}", refusal_word(&reason)))
                        .or_insert(0) += 1;
                    let _ = timed_command(rig, tally, (p.who, "ship/order"), "+ship/order resume");
                    p.retry_at = self.fire + 60;
                    return;
                }
                if docked_at(&rig.tw, hull).as_deref() == Some("ceres_port")
                    && queue_done(&rig.tw, hull)
                {
                    let left = hold_ore(&mut rig.tw, hold);
                    p.loops = p.loops.saturating_add(1);
                    leave_ship(rig, p.who);
                    if left > 0 {
                        sell_hold(rig, tally, p.who, hold);
                    }
                    let dry = plan_loop_dry(rig, p);
                    self.after_loop(p, dry, now_ic);
                }
            }
            Phase::HandLaunch => {
                if !rig_idle(&mut rig.tw, tv1) {
                    return;
                }
                if in_bay(&mut rig.tw, tv1) {
                    let (out, n) =
                        timed_command(rig, tally, (p.who, "tv/launch"), "+tv/1 launch go");
                    if n == 0 {
                        println!("cost_lane: pilot {} launch: {out}", p.index);
                        p.retry_at = self.fire + 30;
                    }
                    return;
                }
                // Out: on to the next field's middle for a fresh reading — the outer blocks
                // first (Occator holds the four around the pad), one a round.
                let fields = ["B", "C", "E", "H", "I", "L", "N", "O", "A", "D", "M", "P"];
                let at =
                    usize::try_from(p.rounds).unwrap_or(0).wrapping_add(p.index) % fields.len();
                let target = match p.claims.first().filter(|_| p.stay_on_claim) {
                    // The month's renewer works its own claim's four fields, one a round.
                    Some(letter) => {
                        let corner =
                            ["NW", "NE", "SW", "SE"][usize::try_from(p.rounds).unwrap_or(0) % 4];
                        format!("{letter}-{corner}")
                    }
                    None => fields[at].to_owned(),
                };
                let line = format!("+tv/1 drive {target} go");
                let _ = timed_command(rig, tally, (p.who, "tv/drive"), &line);
                Self::enter(p, Phase::HandScan, now_ic);
            }
            Phase::HandScan => {
                if !rig_idle(&mut rig.tw, tv1) {
                    return;
                }
                let (out, n) = timed_command(rig, tally, (p.who, "tv/scan"), "+tv/1 scan go");
                if n > 0 || out.contains("nothing new") {
                    Self::enter(p, Phase::HandSound, now_ic);
                } else {
                    p.retry_at = self.fire + 6;
                }
            }
            Phase::HandSound => {
                if !rig_idle(&mut rig.tw, tv1) {
                    return;
                }
                let (out, n) = timed_command(rig, tally, (p.who, "tv/sound"), "+tv/1 sound go");
                if n > 0 {
                    p.heard.clear();
                    Self::enter(p, Phase::HandDrive, now_ic);
                    p.targets.clear();
                    p.sound_pending = true;
                    p.loud_block = rig_block(&rig.tw, tv1);
                } else if out.contains("nothing new") {
                    p.targets.clear();
                    Self::enter(p, Phase::HandRecover, now_ic);
                } else {
                    p.retry_at = self.fire + 6;
                }
            }
            Phase::HandDrive => {
                if !rig_idle(&mut rig.tw, tv1) {
                    return;
                }
                // The sounding's lines are told on the fire after its finish lands: wait for them
                // (an IC hour at most) before reading the targets.
                if p.targets.is_empty()
                    && p.sound_pending
                    && !p.heard.contains(" sounded ")
                    && now_ic - p.since_ic < 3_600
                {
                    return;
                }
                if p.targets.is_empty() && p.sound_pending {
                    p.sound_pending = false;
                    let mut found: Vec<String> = sounded_pockets(&p.heard, "metals")
                        .into_iter()
                        .chain(sounded_pockets(&p.heard, "volatiles"))
                        .filter(|(_, class)| class != "lean")
                        .map(|(addr, _)| addr)
                        .collect();
                    // Never a pocket of a block someone else holds (the public tenure every
                    // player reads on `+registry`).
                    found.retain(|addr| {
                        addr.chars().next().is_none_or(|letter| {
                            block_claim(&mut rig.tw, p.site, letter).is_none_or(|claim| {
                                claim.lapsed_ic.is_some()
                                    || claim.holder == mush_org::PrincipalId::Object(p.who)
                            })
                        })
                    });
                    found.dedup();
                    found.reverse();
                    p.targets = found;
                    p.heard.clear();
                }
                let hopper_load: u64 = {
                    let vault = hopper(&rig.tw, tv1);
                    ORES.iter()
                        .map(|(good, _)| pool(&mut rig.tw, vault, good))
                        .sum()
                };
                let Some(target) = p.targets.pop().filter(|_| hopper_load < 9) else {
                    Self::enter(p, Phase::HandRecover, now_ic);
                    return;
                };
                let line = format!("+tv/1 drive {target} go");
                let (_, n) = timed_command(rig, tally, (p.who, "tv/drive"), &line);
                if n > 0 {
                    Self::enter(p, Phase::HandMine, now_ic);
                }
            }
            Phase::HandMine => {
                if !rig_idle(&mut rig.tw, tv1) {
                    return;
                }
                hand_stake(rig, tally, p);
                let (out, n) = timed_command(rig, tally, (p.who, "tv/mine"), "+tv/1 mine go");
                if n == 0 {
                    *tally
                        .skips
                        .entry(format!("mine.{}", refusal_word(&out)))
                        .or_insert(0) += 1;
                }
                Self::enter(p, Phase::HandDrive, now_ic);
            }
            Phase::HandRecover => {
                if !rig_idle(&mut rig.tw, tv1) {
                    return;
                }
                if !in_bay(&mut rig.tw, tv1) {
                    let (out, n) =
                        timed_command(rig, tally, (p.who, "tv/recover"), "+tv/1 recover go");
                    if n == 0 {
                        println!("cost_lane: pilot {} recover: {out}", p.index);
                        p.retry_at = self.fire + 30;
                    }
                    return;
                }
                p.rounds = p.rounds.saturating_add(1);
                let room = ore_limit_t(&rig.tw).saturating_sub(hold_ore(&mut rig.tw, hold));
                if room >= 12 && p.rounds < 8 {
                    Self::enter(p, Phase::HandLaunch, now_ic);
                } else {
                    Self::enter(p, Phase::HandLift, now_ic);
                }
            }
            Phase::HandLift => {
                if !p.rigs.iter().all(|tv| rig_idle(&mut rig.tw, *tv)) {
                    return;
                }
                let (out, n) = timed_command(rig, tally, (p.who, "ship/lift"), "+ship/lift ceres");
                if n > 0 {
                    Self::enter(p, Phase::Homebound, now_ic);
                } else {
                    println!("cost_lane: pilot {} lift: {out}", p.index);
                    p.retry_at = self.fire + 60;
                }
            }
            Phase::Homebound => {
                if docked_at(&rig.tw, hull).as_deref() == Some("ceres_port") {
                    Self::enter(p, Phase::Sell, now_ic);
                }
            }
            Phase::Sell => {
                leave_ship(rig, p.who);
                let carried = hold_ore(&mut rig.tw, hold);
                sell_hold(rig, tally, p.who, hold);
                p.loops = p.loops.saturating_add(1);
                self.after_loop(p, carried == 0, now_ic);
            }
        }
    }
}

/// The first few words of a refusal, as a skip's kind.
fn refusal_word(out: &str) -> String {
    let words: Vec<String> = out
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
        .take(4)
        .map(str::to_ascii_lowercase)
        .collect();
    words.join("_")
}

/// **The hand pilot's stake** (the brief's L11.3): only when its rig stands in a block it does
/// not already hold and it holds fewer than two live claims; every second hand pilot releases
/// its oldest claim before a third stake; a refusal (`STAKE_YOURS`, `CLAIM_CAP`, claimed,
/// lapsing, cooling) is a skip, counted by kind, and the pilot mines anyway.
fn hand_stake(rig: &mut Rig, tally: &mut CohortTally, p: &mut Pilot) {
    let tv = p.rigs[0];
    let Some(letter) = record_of(&mut rig.tw, tv)
        .and_then(|record| record.excursion)
        .and_then(|excursion| {
            let at = mush_sim::pose_of(rig.tw.world(), tv)?.1;
            let (x, y) = tacsim::surface::site_cell(at.x_cm, at.y_cm);
            let _ = excursion;
            tacsim::surface::address::block_letter(x.div_euclid(4), y.div_euclid(4))
        })
    else {
        return;
    };
    if p.claims.contains(&letter) {
        return;
    }
    p.claims.retain(|held| {
        block_claim(&mut rig.tw, p.site, *held).is_some_and(|claim| {
            claim.lapsed_ic.is_none() && claim.holder == mush_org::PrincipalId::Object(p.who)
        })
    });
    if p.claims.len() >= 2 {
        if p.index % 8 == 3 {
            let oldest = p.claims.remove(0);
            let line = format!("+claim/release {oldest} go");
            let _ = timed_command(rig, tally, (p.who, "claim/release"), &line);
        } else {
            *tally.skips.entry("stake.cap".to_owned()).or_insert(0) += 1;
            return;
        }
    }
    let (out, n) = timed_command(rig, tally, (p.who, "claim/stake"), "+claim/stake go");
    if n > 0 {
        p.claims.push(letter);
    } else {
        *tally
            .skips
            .entry(format!("stake.{}", refusal_word(&out)))
            .or_insert(0) += 1;
    }
}

// ---------------------------------------------------------------------------
// The span's figures (C2's runs, C3's arms, the month)
// ---------------------------------------------------------------------------

/// One arm's per-arm maxima: `[visited, worked, deferred, held, refused]`.
type ArmMax = [u32; 5];

fn arm_max(into: &mut ArmMax, arm: &tacsim::port_tick::ArmFireStats) {
    for (slot, value) in
        into.iter_mut()
            .zip([arm.visited, arm.worked, arm.deferred, arm.held, arm.refused])
    {
        *slot = (*slot).max(value);
    }
}

/// **What a measured span recorded**, fire by fire (the brief's L6 figures).
#[derive(Clone, Debug, Default)]
struct Span {
    /// Each surface fire's own wall time, µs.
    surface_us: Vec<u64>,
    /// Each port fire's own wall time, µs.
    port_us: Vec<u64>,
    /// Each second's whole `fire_due`, µs.
    fire_us: Vec<u64>,
    /// Each second's other ticks' share (`fire_us` less the surface and port fires), µs.
    other_us: Vec<u64>,
    /// Warms by kind: count and the largest `warm_us`.
    warmed: BTreeMap<String, (u64, u64)>,
    /// The fire indexes of the site and home warms, with their µs.
    warm_fires: Vec<(usize, String, u64)>,
    searches: u64,
    search_us: u64,
    /// Per-search µs on the fires that searched.
    per_leg: Vec<u64>,
    peeks: u64,
    peek_us: u64,
    /// Per-peek µs on the fires that peeked.
    per_peek: Vec<u64>,
    markers_held: u64,
    /// The surface fire's maxima: visited, worked, passed over.
    surface_max: [u32; 3],
    /// The port fire's total maxima: visited, worked, deferred, held, refused.
    port_max: ArmMax,
    /// The port fire's maxima by arm: stock (five clerks), site, dispatch.
    stock_max: ArmMax,
    site_max: ArmMax,
    dispatch_max: ArmMax,
    /// The most submissions one second queued.
    queued_max: usize,
    /// Deferrals, surface and port.
    deferrals: u64,
    /// Fires that visited a rig yet read 0 µs.
    zero_elapsed: u64,
    /// Fires.
    fires: u64,
}

impl Span {
    fn record(&mut self, fired: &Fired) {
        let index = usize::try_from(self.fires).unwrap_or(usize::MAX);
        self.fires = self.fires.saturating_add(1);
        self.fire_us.push(fired.fire_us);
        let mut own = 0_u64;
        if let Some(stats) = fired.surface {
            self.surface_us.push(stats.elapsed_us);
            own = own.saturating_add(stats.elapsed_us);
            if stats.visited >= 1 && stats.elapsed_us == 0 {
                self.zero_elapsed += 1;
            }
            let word = format!("{:?}", stats.warmed);
            let entry = self.warmed.entry(word.clone()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.max(stats.warm_us);
            if matches!(stats.warmed, Warmed::Site | Warmed::Home) {
                self.warm_fires.push((index, word, stats.warm_us));
            }
            self.searches += u64::from(stats.searches);
            self.search_us += stats.search_us;
            if stats.searches > 0 {
                self.per_leg
                    .push(stats.search_us / u64::from(stats.searches));
            }
            self.peeks += u64::from(stats.peeks);
            self.peek_us += stats.peek_us;
            if stats.peeks > 0 {
                self.per_peek.push(stats.peek_us / u64::from(stats.peeks));
            }
            self.markers_held += u64::from(stats.markers_held);
            self.surface_max[0] = self.surface_max[0].max(stats.visited);
            self.surface_max[1] = self.surface_max[1].max(stats.worked);
            self.surface_max[2] = self.surface_max[2].max(stats.passed_over);
            self.deferrals += u64::from(stats.deferred);
        }
        if let Some(stats) = fired.port {
            self.port_us.push(stats.elapsed_us);
            own = own.saturating_add(stats.elapsed_us);
            arm_max(
                &mut self.port_max,
                &tacsim::port_tick::ArmFireStats {
                    visited: stats.visited,
                    worked: stats.worked,
                    deferred: stats.deferred,
                    held: stats.held,
                    refused: stats.refused,
                },
            );
            arm_max(&mut self.stock_max, &stats.stock);
            arm_max(&mut self.site_max, &stats.site);
            arm_max(&mut self.dispatch_max, &stats.dispatch);
            self.deferrals += u64::from(stats.deferred);
        }
        self.other_us.push(fired.fire_us.saturating_sub(own));
        self.queued_max = self.queued_max.max(fired.queued.len());
    }

    /// The `cost_lane:` lines of the span.
    fn print(&self, head: &str, ops_per_fire: &[u64]) {
        let fig = |name: &str, samples: &[u64]| {
            println!(
                "cost_lane: {head} {name} p50 {} p99 {} max {} us (n {})",
                cohort::p50(samples),
                cohort::p99(samples),
                cohort::max(samples),
                samples.len()
            );
        };
        fig("surface fire", &self.surface_us);
        println!(
            "cost_lane: {head} surface fire five-window p99s {:?} us",
            cohort::windows(&self.surface_us, 5)
        );
        fig("port fire", &self.port_us);
        fig("fire_due", &self.fire_us);
        fig("other ticks", &self.other_us);
        fig("search per leg", &self.per_leg);
        fig("peek per peek", &self.per_peek);
        println!(
            "cost_lane: {head} warms {:?}; searches {} ({} us); peeks {} ({} us); markers_held \
             {} (hold decisions)",
            self.warmed, self.searches, self.search_us, self.peeks, self.peek_us, self.markers_held
        );
        println!(
            "cost_lane: {head} surface max visited/worked/passed_over {:?} (caps 64/16); port \
             max visited/worked/deferred/held/refused {:?}; stock arm (five clerks) {:?}; site \
             {:?}; dispatch {:?} (the rig lands every submission between fires, so Pending \
             deferrals are near zero by construction)",
            self.surface_max, self.port_max, self.stock_max, self.site_max, self.dispatch_max
        );
        println!(
            "cost_lane: {head} queued per second max {} (queue max {}); deferrals {}; module \
             ops per fire p99 {} max {}",
            self.queued_max,
            mush_core::AUTHORITATIVE_SUBMIT_QUEUE_MAX,
            self.deferrals,
            cohort::p99(ops_per_fire),
            cohort::max(ops_per_fire)
        );
    }
}

// ---------------------------------------------------------------------------
// C2 — the surface tick's gate on the shipped lane
// ---------------------------------------------------------------------------

/// C2's cohort: four pilots and their sites (three fly plans, the fourth plays by hand).
const C2_PILOTS: [(&str, i64, &str, &str, &str); 4] = [
    ("Ada", 1, "Salt Moth", "ahuna", AHUNA),
    ("Bryn", 2, "Grey Heron", "juling", "juling_port"),
    ("Cass", 3, "Tern", "kerwan", "kerwan_port"),
    ("Dov", 4, "Kite", "occator", OCCATOR),
];

/// C2's measured fires: 12 IC hours at rate 24.
const C2_FIRES: u32 = 1_800;

/// **What the players must land in C2's span, at least** (the review's tests/T2): the three plan
/// pilots' filings on both rigs (`start.plan_file`, 3 × 2), a cycle (`done.cycle`), and the hand
/// pilot's own run (`start.mine`, a hand start, never a plan's).
const C2_PLAYED: [(&str, u64); 3] = [("start.plan_file", 6), ("done.cycle", 1), ("start.mine", 1)];

/// The landings of `kind` attributed to a player so far.
fn player_landed(rig: &Rig, kind: &str) -> u64 {
    rig.tally.kinds.get(kind).map_or(0, |row| row.player)
}

/// **C2's world** (the brief's L6): the shipped lane at rate 24, four Dray-P pilots of two rigs
/// each settled on the pads of Ahuna Flank, Juling Shadow, Kerwan Floor and Occator Rim beside the
/// yard's two rigs — ten TVs, the most the shipped lane fields; the cohort ready to play.
fn c2_world() -> (Rig, Cohort) {
    let mut rig = stocked(Roster::default());
    let mut cohort = Cohort::default();
    for (index, (who, seed, name, venue, entry)) in C2_PILOTS.into_iter().enumerate() {
        let (pilot, row) = kit_up(&mut rig, (who, seed, name), 2);
        board(&mut rig, pilot);
        fly_to(&mut rig, pilot, &row, (venue, entry));
        let rigs = rigs_of(&rig.tw, pilot);
        let mode = if index == 3 { Mode::Hand } else { Mode::Plan };
        cohort.pilots.push(Pilot {
            index,
            who: pilot,
            row,
            rigs,
            site: entry,
            venue,
            mode,
            family: if index % 2 == 0 {
                "metal_ore"
            } else {
                "ice_ore"
            },
            phase: match mode {
                Mode::Plan => Phase::PlanScan,
                Mode::Hand => Phase::HandLaunch,
            },
            heard: String::new(),
            targets: Vec::new(),
            claims: Vec::new(),
            loops: 0,
            since_ic: ic_now(&rig.tw),
            start_at: 0,
            rounds: 0,
            sound_pending: false,
            stay_on_claim: false,
            loud_block: None,
            retry_at: 0,
            bunker_rounds: 0,
            filed: false,
            dry: false,
            dry_sites: BTreeSet::new(),
            dry_families: BTreeSet::new(),
            filed_family: None,
            progress_ic: ic_now(&rig.tw),
            progress_sig: (0, 0),
        });
    }
    for _ in 0..12 {
        let _ = fire(&mut rig);
    }
    (rig, cohort)
}

/// **C2 — the surface tick is measured** (doc 20 §11's Tick row "p99 < 5 ms a fire"; doc 20a
/// §5.3.5; AP's idiom): C2's cohort plays 1,800 one-second fires (12 IC hours at rate 24) —
/// three pilots scan by hand and fly fill plans, the fourth works by hand — commands and landings
/// between fires, never inside a fire's clock. **In release** and under `TACSIM_ASSERT=1` five
/// runs, each from a fresh boot; **in plain debug** once. The gates (O-25): release, the minimum
/// over five runs of each run's surface-fire p99 under 5,000 µs, always; debug under
/// `TACSIM_ASSERT=1`, the minimum p99 at most 50 ms and the minimum max at most 250 ms; plain
/// debug (the push shard) one run's p99 at most 500 ms and max at most 2,500 ms. Always: legs
/// searched, every fire that visited a rig timed, no failed warm, the world rate 24 at both ends,
/// no refused write, every kind classified. **The span holds no cold site**: the site clerks want
/// every lane site's ground at a fresh boot and six never evict from sixteen, so a site build's
/// cost is C4's, the restart's warm C3's and the month's; the span's warms are printed.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12c_the_surface_tick_is_measured() {
    let _lock = exclusive();
    let started = Instant::now();
    let asserted = asserting();
    let runs = if cfg!(debug_assertions) && !asserted {
        1
    } else {
        5
    };
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let (mut p99s, mut maxes) = (Vec::new(), Vec::new());
    for run_no in 0..runs {
        let (mut rig, mut cohort) = c2_world();
        assert_eq!(world_rate(&rig.tw), RATE);
        let counters = tacsim::surface::site_cache_counters(rig.tw.world());
        println!(
            "cost_lane: C2 run {run_no} ({build}) world rate {} ephemeris rate 0; at the span's \
             start the site cache holds {} (builds {}, evictions {})",
            world_rate(&rig.tw),
            counters.entries,
            counters.builds,
            counters.evictions
        );
        let ic0 = ic_now(&rig.tw);
        rig.ops_per_fire.clear();
        let played0: Vec<u64> = C2_PLAYED
            .iter()
            .map(|(kind, _)| player_landed(&rig, kind))
            .collect();
        let mut span = Span::default();
        for _ in 0..C2_FIRES {
            let fired = second(&mut rig);
            span.record(&fired);
            cohort.step(&mut rig, &fired);
        }
        // **The cohort played** (the review's tests/T2: the yard alone searches legs, so the
        // searches below prove nothing of the players): in the span, the three plan pilots filed
        // both rigs' plans, the players' rigs cycled, and the hand pilot started a run.
        for ((kind, least), before) in C2_PLAYED.iter().zip(&played0) {
            let played = player_landed(&rig, kind).saturating_sub(*before);
            println!("cost_lane: C2 run {run_no} players' {kind} in the span: {played}");
            assert!(
                played >= *least,
                "C2: the players' {kind} landed {played} times in the span, under {least}"
            );
        }
        assert_eq!(world_rate(&rig.tw), RATE);
        let ic_span = ic_now(&rig.tw) - ic0;
        println!(
            "cost_lane: C2 run {run_no} IC span {ic_span} s ({} h)",
            ic_span / 3_600
        );
        assert_eq!(
            ic_span,
            i64::from(C2_FIRES) * i64::from(RATE),
            "12 IC hours"
        );
        span.print(&format!("C2 run {run_no} ({build})"), &rig.ops_per_fire);
        println!(
            "cost_lane: C2 run {run_no} pilots {:?}; skips {:?}",
            cohort
                .pilots
                .iter()
                .map(|p| (p.index, format!("{:?}", p.phase), p.loops))
                .collect::<Vec<_>>(),
            cohort.tally.skips
        );
        for (verb, samples) in &cohort.tally.timed {
            println!(
                "cost_lane: C2 run {run_no} command {verb} p50 {} p99 {} max {} us (n {})",
                cohort::p50(samples),
                cohort::p99(samples),
                cohort::max(samples),
                samples.len()
            );
        }
        // Always.
        assert!(
            span.searches > 0 && span.search_us > 0,
            "the plans searched legs"
        );
        assert_eq!(
            span.zero_elapsed, 0,
            "every fire that visited a rig is timed"
        );
        assert!(!span.warmed.contains_key("Failed"), "no failed warm");
        if counters.entries < 6 {
            assert!(
                span.warm_fires
                    .iter()
                    .any(|(_, word, us)| word == "Site" && *us > 0),
                "a cold site at the span's start is warmed in it: {:?}",
                span.warm_fires
            );
        }
        print_tally(&rig, "C2");
        assert_within_gates(&rig);
        assert_kinds_classified(&rig);
        no_refused_writes(&rig.tw);
        p99s.push(cohort::p99(&span.surface_us));
        maxes.push(cohort::max(&span.surface_us));
    }
    let (min_p99, min_max) = (
        p99s.iter().copied().min().unwrap_or(0),
        maxes.iter().copied().min().unwrap_or(0),
    );
    println!(
        "cost_lane: C2 ({build}{}) p99s {p99s:?} max {maxes:?}: the gate's minimum p99 {min_p99} \
         us, minimum max {min_max} us; wall {:.1} s",
        if asserted { ", TACSIM_ASSERT=1" } else { "" },
        started.elapsed().as_secs_f64()
    );
    if !cfg!(debug_assertions) {
        assert!(
            min_p99 < 5_000,
            "the surface tick's release gate: the minimum p99 {min_p99} us of {p99s:?}"
        );
    } else if asserted {
        assert!(
            min_p99 <= 50_000,
            "debug under TACSIM_ASSERT: p99 {min_p99} us"
        );
        assert!(
            min_max <= 250_000,
            "debug under TACSIM_ASSERT: max {min_max} us"
        );
    } else {
        assert!(min_p99 <= 500_000, "plain debug: p99 {min_p99} us");
        assert!(min_max <= 2_500_000, "plain debug: max {min_max} us");
    }
}

// ---------------------------------------------------------------------------
// C3 — a restart inside the cohort changes nothing
// ---------------------------------------------------------------------------

/// The one-second fires arm B's warm may take (one site or one home a fire; C2's cohort has rigs
/// out at four sites, so about eight).
const C3_WARM_MAX: u32 = 12;
/// The jump past the reboot point, real seconds: at least the longest periodic tick period of the
/// composition plus the warm, so one fire at the jump re-aligns every tick in both arms (asserted
/// against the registry at the test's start). P1's 120 covered the 60 s sweeps the brief named
/// as the longest; the shipped composition also registers the weather tick at 300 s, so the window
/// is raised to cover it (the brief's own rule: "if a longer one exists, raise C3_WINDOW").
const C3_WINDOW: i64 = 320;
/// The span the reboot point is taken in, fire indexes.
const C3_POINTS: std::ops::RangeInclusive<u32> = 300..=450;
/// The fires each arm runs, counted from the start (the warm and the jump not counted).
const C3_FIRES: u32 = 600;

/// The lane's yard rigs, slot order.
fn yard_rigs(tw: &TestWorld) -> Vec<ObjId> {
    let mut rigs = yard(tw).rigs;
    rigs.sort_by_key(|rig| rig.n);
    rigs.into_iter().map(|rig| ObjId(rig.tv)).collect()
}

/// The ice-ore unlisted and the mill ledger of `org`'s mill of `ore`.
fn mill_ledger(tw: &mut TestWorld, org: &str, ore: &str) -> Option<tacsim::port::mill::MillLedger> {
    let id = tacsim::port::mill::MillId::derive(
        &cid(LANE_PACK),
        &cid("ceres_port"),
        &cid(org),
        &cid(ore),
    )
    .ok()?;
    let h = handles(tw);
    typed(tw, &h.families.mills, &id)
}

/// The units a mill made and its vendor has not yet listed.
fn unlisted(tw: &mut TestWorld, org: &str, ore: &str) -> u64 {
    let product = mill_ledger(tw, org, ore).map_or(0, |ledger| ledger.product_t);
    let counter = port_map(tw, "ceres_port")
        .vendors
        .get(&cid(org))
        .and_then(|row| row.mill_listed.get(&cid(ore)).copied())
        .unwrap_or(0);
    product.saturating_sub(counter)
}

/// The lane's two mills: the org and the ore it mills.
const MILLS: [(&str, &str); 2] = [("freeport_exchange", "metal_ore"), ("occator", "ice_ore")];
/// The shipped mills' batch and daily rate, tonnes of ore (lane v15, SM.12a).
const SHIPPED_MILL: (u64, u64) = (20, 40);

/// **Whether a mill's batch is due** (`mill::mill_step`'s rule, T9's): its storefront holds at
/// least a batch of its ore and the IC day's room is not spent (`day_ore_t + batch_t` within
/// `per_ic_day_t` on the ledger's day).
fn mill_batch_due(
    tw: &mut TestWorld,
    (org, ore): (&str, &str),
    (batch_t, per_day_t): (u64, u64),
) -> bool {
    let day = ic_now(tw).div_euclid(DAY);
    let at = storefront(tw, org);
    let ore_t = pool(tw, at, ore);
    let spent = mill_ledger(tw, org, ore)
        .is_some_and(|ledger| ledger.day == day && ledger.day_ore_t + batch_t > per_day_t);
    ore_t >= batch_t && !spent
}

/// **The Ceres clerk's steps due now** (the brief's L3.9; the review's correctness/F7 and
/// conformance/F2), each re-pinned from `stock::stock_step`'s own rules, the mills' batch and
/// rate as the run's lane authors them:
/// - `batch`: [`mill_batch_due`] for either mill;
/// - `restock`: a mill's product made and not yet listed ([`unlisted`] > 0);
/// - `list`: a vendor's stock good in its storefront with no ask listed (the list step relists
///   whatever is gone: sold out, lapsed, withdrawn);
/// - `drip`: an ore bid posted on an earlier IC day whose standing listing is under its intake
///   (`stock::drip_step`'s case 3, the lane authoring every ore bid's cap at its intake), or gone
///   past its recorded deadline (case 4).
fn clerk_due(rig: &mut Rig, mill: (u64, u64)) -> Vec<&'static str> {
    let mut due = Vec::new();
    if MILLS
        .iter()
        .any(|(org, ore)| mill_batch_due(&mut rig.tw, (org, ore), mill))
    {
        due.push("batch");
    }
    if MILLS
        .iter()
        .any(|(org, ore)| unlisted(&mut rig.tw, org, ore) > 0)
    {
        due.push("restock");
    }
    let map = port_map(&rig.tw, "ceres_port");
    let mut missing = false;
    for (org, row) in &map.vendors {
        for stock in &row.stock {
            if ask_of(&rig.tw, org.as_str(), stock.good.as_str()).is_none()
                && pool(&mut rig.tw, VaultId(row.vault), stock.good.as_str()) > 0
            {
                missing = true;
            }
        }
    }
    if missing {
        due.push("list");
    }
    let today = ic_now(&rig.tw).div_euclid(DAY);
    let now = rig.tw.world().now();
    let drip = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .is_some_and(|market| {
            map.vendors.values().any(|row| {
                row.bids.iter().any(|(good, bid)| {
                    let boundary = bid
                        .posted_ic
                        .is_none_or(|posted| posted.div_euclid(DAY) < today);
                    is_ore(good)
                        && match market.listing(mush_market::ListingId(bid.listing)) {
                            Some(listing) => boundary && listing.quantity < bid.intake,
                            None => bid.expires <= now,
                        }
                })
            })
        });
    if drip {
        due.push("drip");
    }
    due
}

/// **Whether nothing the restart could perturb falls due during arm B's warm** (the brief's L7):
/// `Err` names the clause and the first rig or agent it blocked on.
#[allow(clippy::too_many_lines)]
fn cohort_quiet(
    rig: &mut Rig,
    cohort: &Cohort,
    recent_clerk: bool,
) -> Result<(), (&'static str, String)> {
    let rate = i64::from(world_rate(&rig.tw));
    let now = ic_now(&rig.tw);
    let horizon = now + i64::from(C3_WARM_MAX + 2) * rate;
    let mut rigs: Vec<ObjId> = cohort.pilots.iter().flat_map(|p| p.rigs.clone()).collect();
    let yard_tvs = yard_rigs(&rig.tw);
    rigs.extend(yard_tvs.iter().copied());
    for tv in rigs {
        if tacsim::surface::tick::plan_wait_for_tests(rig.tw.world(), tv).is_some() {
            return Err(("a cached wait", format!("#{}", tv.0)));
        }
        let Some(record) = record_of(&mut rig.tw, tv) else {
            continue;
        };
        if record.queued.is_some() {
            return Err(("a queued unload", format!("#{}", tv.0)));
        }
        // A start landed whose projection the tick has not yet written (it writes it on the next
        // fire, which in arm B is a warm fire): tightened after the backstop caught one.
        if let Some(tacsim::surface::TvRecState::Present(rec)) = store(&rig.tw).tv_rec(tv) {
            let plan_seq = record.plan.as_ref().map(|plan| plan.seq);
            if rec.run.is_some() != record.run.is_some()
                || rec.act.is_some() != record.act.is_some()
                || rec.told < rec.applied
                || (plan_seq.is_some() && rec.plan_told.map(|told| told.seq) != plan_seq)
            {
                return Err(("a projection to write", format!("#{}", tv.0)));
            }
        }
        // The rig lands every submission between fires, so nothing is ever in flight here.
        assert_eq!(
            rig.tw.engine().queued_authoritative_actors().count(),
            0,
            "a submission in flight at a candidate point"
        );
        let due = record
            .act
            .as_ref()
            .map(|act| act.due_ic)
            .or(record.run.as_ref().map(|run| run.due_ic));
        match due {
            Some(due) if due > horizon => {}
            Some(_) => return Err(("an act due in the warm", format!("#{}", tv.0))),
            None => {
                let live = record
                    .plan
                    .as_ref()
                    .is_some_and(tacsim::surface::plan::FillPlan::live);
                if live {
                    return Err(("an idle rig with a live plan", format!("#{}", tv.0)));
                }
                if let Some(tacsim::surface::plan::FillState::Ended { why, at_ic }) =
                    record.plan.as_ref().map(|plan| plan.state)
                    && yard_tvs.contains(&tv)
                    && matches!(
                        why,
                        tacsim::surface::plan::PlanEnd::HoldFull
                            | tacsim::surface::plan::PlanEnd::Bound
                    )
                    && at_ic + 3_600 <= horizon
                {
                    return Err(("a re-arm due in the warm", format!("#{}", tv.0)));
                }
                if record.excursion.is_some()
                    && record.idle_since().is_some_and(|since| {
                        since + tacsim::surface::plan::IDLE_PARK_IC_SECS <= horizon
                    })
                {
                    return Err(("an idle return due in the warm", format!("#{}", tv.0)));
                }
            }
        }
    }
    // The port: a delivery, a batch or a restock due; a Ceres step; a day boundary.
    let front = storefront(&rig.tw, "occator");
    let yard_hold = VaultId(yard(&rig.tw).hold.expect("the yard hold"));
    let front_ice = pool(&mut rig.tw, front, "ice_ore");
    if pool(&mut rig.tw, yard_hold, "ice_ore") > 0 && front_ice < 40 {
        return Err(("a delivery due", "the dispatcher".to_owned()));
    }
    let day = now.div_euclid(DAY);
    for (org, ore) in MILLS {
        if mill_batch_due(&mut rig.tw, (org, ore), SHIPPED_MILL) {
            return Err(("a batch due", org.to_owned()));
        }
        if unlisted(&mut rig.tw, org, ore) > 0 {
            return Err(("a restock due", org.to_owned()));
        }
    }
    if recent_clerk {
        return Err(("a Ceres step due", "the Ceres clerk".to_owned()));
    }
    if day != horizon.div_euclid(DAY) {
        return Err(("a day boundary", format!("day {day}")));
    }
    // The hulls: a passage arriving, or a queue whose next step is ready.
    for p in &cohort.pilots {
        let hull = ObjId(p.row.hull);
        if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::UnderWay
        ) {
            return Err(("a hull step due", format!("pilot {} under way", p.index)));
        }
        if !queue_done(&rig.tw, hull) && p.rigs.iter().all(|tv| !busy(&mut rig.tw, *tv)) {
            return Err(("a hull step due", format!("pilot {} queue ready", p.index)));
        }
    }
    Ok(())
}

/// **The post-image, by name** (the brief's L3.8): every rig record and projection, every chart
/// row of the cohort's pilots and the dispatcher at each site, every block's tenure record and
/// deposit cursor, every pool of every hold, hopper, block vault, spoil vault, the yard hold and
/// the Ceres storefronts, every economy account, every market listing, both mill ledgers, the
/// dispatch ledger, the Ceres and Occator Rim port maps and the world's IC clock.
#[allow(clippy::too_many_lines)]
fn post_image(rig: &mut Rig, cohort: &Cohort) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut rigs: Vec<ObjId> = cohort.pilots.iter().flat_map(|p| p.rigs.clone()).collect();
    rigs.extend(yard_rigs(&rig.tw));
    for tv in &rigs {
        out.insert(
            format!("record #{}", tv.0),
            format!("{:?}", record_of(&mut rig.tw, *tv)),
        );
        let rec = match store(&rig.tw).tv_rec(*tv) {
            Some(tacsim::surface::TvRecState::Present(rec)) => format!("{rec:?}"),
            other => format!("{:?}", other.is_some()),
        };
        out.insert(format!("projection #{}", tv.0), rec);
    }
    let dispatcher = ObjId(yard(&rig.tw).dispatcher.expect("a dispatcher"));
    let readers: Vec<ObjId> = cohort
        .pilots
        .iter()
        .map(|p| p.who)
        .chain(std::iter::once(dispatcher))
        .collect();
    for site in SITES {
        for who in &readers {
            out.insert(
                format!("chart #{} {site}", who.0),
                format!("{:?}", chart_of(&mut rig.tw, *who, site)),
            );
        }
        for letter in tacsim::port::def::BLOCK_LETTERS {
            out.insert(
                format!("block {site} {letter}"),
                format!("{:?}", block_record(&mut rig.tw, site, letter)),
            );
            for family in tacsim::surface::geology::OreFamily::ALL {
                let Ok(id) = mush_industry::derive_system_deposit_id(
                    &tacsim::surface::opening::receipt(&cid(site), letter, family),
                ) else {
                    continue;
                };
                let pockets = handles(&rig.tw).families.pockets;
                out.insert(
                    format!("cursor {site} {letter} {family:?}"),
                    format!("{:?}", typed(&mut rig.tw, &pockets, &id)),
                );
            }
        }
    }
    let mut vaults: Vec<(String, VaultId)> = Vec::new();
    for p in &cohort.pilots {
        vaults.push((
            format!("hold of pilot {}", p.index),
            hold_vault(&rig.tw, &p.row),
        ));
    }
    for tv in &rigs {
        vaults.push((format!("hopper #{}", tv.0), hopper(&rig.tw, *tv)));
    }
    for site in SITES {
        let map = port_map(&rig.tw, site);
        if let Some(row) = map.site {
            for (letter, vault) in row.vaults {
                vaults.push((format!("block vault {site} {letter}"), VaultId(vault)));
            }
            if let Some(spoil) = row.spoil_vault {
                vaults.push((format!("spoil {site}"), VaultId(spoil)));
            }
        }
    }
    vaults.push((
        "the yard hold".to_owned(),
        VaultId(yard(&rig.tw).hold.expect("the yard hold")),
    ));
    for (org, row) in port_map(&rig.tw, "ceres_port").vendors {
        vaults.push((format!("storefront {org}"), VaultId(row.vault)));
    }
    let goods = [
        "metal_ore",
        "ice_ore",
        "carbon_ore",
        "strategic_ore",
        "metals",
        "volatiles",
        "propellant",
        "provisions",
    ];
    for (name, vault) in vaults {
        for good in goods {
            out.insert(
                format!("pool {name} {good}"),
                pool(&mut rig.tw, vault, good).to_string(),
            );
        }
    }
    if let Some(economy) = rig.tw.world().module_state::<mush_economy::EconomyStore>() {
        for account in economy.snapshot().accounts() {
            out.insert(
                format!("account {}", account.id.0),
                format!(
                    "{} {}",
                    account.posted_balance.widen(),
                    account.held_amount.widen()
                ),
            );
        }
    }
    if let Some(market) = rig.tw.world().module_state::<mush_market::MarketStore>() {
        for listing in market.live_listings() {
            out.insert(
                format!("listing {}", listing.id.0),
                format!("{} {}", listing.quantity, listing.revision),
            );
        }
    }
    for (org, ore) in [("freeport_exchange", "metal_ore"), ("occator", "ice_ore")] {
        out.insert(
            format!("mill {org}"),
            format!("{:?}", mill_ledger(&mut rig.tw, org, ore)),
        );
    }
    let h = handles(&rig.tw);
    out.insert(
        "dispatch ledger".to_owned(),
        format!(
            "{:?}",
            typed(
                &mut rig.tw,
                &h.families.dispatch,
                &tacsim::port::yard::DispatchKey { site: cid(OCCATOR) }
            )
        ),
    );
    out.insert(
        "map ceres".to_owned(),
        format!("{:?}", port_map(&rig.tw, "ceres_port")),
    );
    out.insert(
        "map occator".to_owned(),
        format!("{:?}", port_map(&rig.tw, OCCATOR)),
    );
    out.insert("ic".to_owned(), ic_now(&rig.tw).to_string());
    out
}

/// What one C3 arm ends with: the post-image, the reboot point, and the restart's warm figures.
struct ArmEnd {
    image: BTreeMap<String, String>,
    point: u32,
    warm: Vec<(u32, String, u64)>,
    after_us: u64,
    /// The players' landings from the point to the arm's end (the review's tests/T2).
    played_after: u64,
}

/// **One arm of C3** (the brief's L7): C2's cohort, one-second fires from the start; arm A finds
/// the reboot point (`point` `None`), arm B runs to the given one; at the point arm B reboots, both
/// warm and jump to one instant past every tick's period and fire once there; then both run on to
/// fire 600.
#[allow(clippy::too_many_lines)]
fn c3_arm(point: Option<u32>) -> ArmEnd {
    let (mut rig, mut cohort) = c2_world();
    let longest = rig
        .tw
        .engine()
        .registry
        .module_ticks()
        .iter()
        .filter_map(|tick| match tick.cadence {
            mush_core::TickCadence::Coarse(period) | mush_core::TickCadence::Fast(period) => {
                Some((
                    tick.module.to_string(),
                    tick.tag.to_string(),
                    period.as_secs(),
                ))
            }
            mush_core::TickCadence::Scene => None,
        })
        .collect::<Vec<_>>();
    if point.is_none() {
        println!("cost_lane: C3 periodic ticks {longest:?}");
    }
    let most = longest.iter().map(|(_, _, secs)| *secs).max().unwrap_or(0);
    assert!(
        C3_WINDOW > i64::try_from(most).unwrap_or(i64::MAX) + i64::from(C3_WARM_MAX),
        "C3_WINDOW covers the longest periodic tick ({most} s) and the warm"
    );
    let ic0 = ic_now(&rig.tw);
    let mut counts: BTreeMap<&'static str, (u32, String)> = BTreeMap::new();
    let mut taken = None;
    let mut recent: Vec<bool> = Vec::new();
    let mut n = 0_u32;
    let rebooting = point.is_some();
    let mut warm = Vec::new();
    let mut after_us = 0_u64;
    let mut after_reboot = false;
    let mut jumped = false;
    let mut players_at_point = 0_u64;
    while n < C3_FIRES {
        let at_point = match point {
            Some(p) => n == p && !jumped,
            None if taken.is_none() && C3_POINTS.contains(&n) => {
                let recent_clerk = recent.iter().rev().take(3).any(|acted| *acted);
                match cohort_quiet(&mut rig, &cohort, recent_clerk) {
                    Ok(()) => {
                        taken = Some(n);
                        true
                    }
                    Err((clause, who)) => {
                        let entry = counts.entry(clause).or_insert((0, who));
                        entry.0 += 1;
                        false
                    }
                }
            }
            None => false,
        };
        if point.is_none() && taken.is_none() && n == *C3_POINTS.end() + 1 {
            panic!(
                "C3: no quiet point in fires 300..=450: {counts:?} (the clause, its count and \
                 its first blocker)"
            );
        }
        if at_point {
            if rebooting {
                let recent_clerk = recent.iter().rev().take(3).any(|acted| *acted);
                assert_eq!(
                    cohort_quiet(&mut rig, &cohort, recent_clerk),
                    Ok(()),
                    "arm B's point is quiet too (the arms are identical up to it)"
                );
            }
            let at = rig.tw.world().now() + C3_WINDOW;
            players_at_point = rig.tally.player_landings();
            if rebooting {
                let _ = count_and_fold(&mut rig);
                assert!(count_and_fold(&mut rig).is_empty(), "nothing more to fold");
                rig = reboot(rig);
                // The boot pass's own documents (the modules' meta rows), folded before the reads.
                let _ = count_and_fold(&mut rig);
                after_reboot = true;
            }
            // Both arms: every pilot with a rig out reads `+tv`, the dispatcher its site map.
            for p in &cohort.pilots {
                if p.rigs.iter().any(|tv| !in_bay(&mut rig.tw, *tv)) {
                    let _ = run(&mut rig.tw, p.who, "+tv");
                }
            }
            let dispatcher = ObjId(yard(&rig.tw).dispatcher.expect("a dispatcher"));
            let _ = run(&mut rig.tw, dispatcher, "+tv/map site");
            let read_ops = count_and_fold(&mut rig);
            assert!(read_ops.is_empty(), "the reads write nothing: {read_ops:?}");
            let sites: BTreeSet<&str> = cohort
                .pilots
                .iter()
                .filter(|p| p.rigs.iter().any(|tv| !in_bay(&mut rig.tw, *tv)))
                .map(|p| p.site)
                .chain(std::iter::once(OCCATOR))
                .collect();
            let mut k = 0_u32;
            while !sites.iter().all(|site| warm_at(&rig.tw, site)) {
                assert!(
                    k < C3_WARM_MAX,
                    "C3: the warm took more than {C3_WARM_MAX} fires"
                );
                let fired = second(&mut rig);
                k += 1;
                if let Some(stats) = fired.surface {
                    warm.push((k, format!("{:?}", stats.warmed), stats.warm_us));
                }
                assert!(
                    fired.queued.is_empty()
                        && fired.ops.iter().all(|op| !matches!(
                            op.ns.as_str(),
                            "tacsim" | "inventory" | "industry" | "market"
                        )),
                    "C3: the point was not quiet: fire {k} wrote {:?} / queued {:?}; tighten \
                     cohort_quiet",
                    fired.ops,
                    fired
                        .queued
                        .iter()
                        .map(|s| s.action.key().id.to_string())
                        .collect::<Vec<_>>()
                );
            }
            if !rebooting {
                assert_eq!(k, 0, "arm A's sites are all warm already");
            }
            assert!(rig.tw.world().now() < at, "the warm ran past the point");
            let fired = step_to(&mut rig, at);
            for stats in [fired.surface.map(|s| (s.visited, s.worked))]
                .into_iter()
                .flatten()
            {
                assert!(
                    stats.0 < 64 && stats.1 < 16,
                    "the jump's surface fire reached a cap"
                );
            }
            if let Some(port) = fired.port {
                assert!(
                    port.visited < 64 && port.worked < 16,
                    "the jump's port fire reached a cap"
                );
            }
            cohort.step(&mut rig, &fired);
            // The jump and the warm fires are not counted.
            jumped = true;
            continue;
        }
        let fired = second(&mut rig);
        if let Some(port) = fired.port {
            assert!(
                port.visited < 64 && port.worked < 16,
                "fire {n}: a port fire reached a cap: {port:?}"
            );
            recent.push(fired.clerk.is_some());
        }
        if after_reboot && let Some(stats) = fired.surface {
            after_us = after_us.max(stats.elapsed_us);
        }
        cohort.step(&mut rig, &fired);
        n += 1;
    }
    let point = point.or(taken).expect("a reboot point");
    let ic_span = ic_now(&rig.tw) - ic0;
    println!(
        "cost_lane: C3 arm {} reboot point {point}; per-clause counts {counts:?}; IC span {ic_span} \
         s",
        if rebooting { "B" } else { "A" }
    );
    assert_eq!(world_rate(&rig.tw), RATE);
    assert_eq!(
        ic_span,
        i64::from(C3_FIRES) * i64::from(RATE) + C3_WINDOW * i64::from(RATE),
        "four IC hours, plus the window"
    );
    assert_kinds_classified(&rig);
    no_refused_writes(&rig.tw);
    assert_no_page(&rig, "C3");
    let played_after = rig.tally.player_landings().saturating_sub(players_at_point);
    let image = post_image(&mut rig, &cohort);
    ArmEnd {
        image,
        point,
        warm,
        after_us,
        played_after,
    }
}

/// **C3 — a restart inside the cohort changes nothing** (doc 20 §8 "a restart changes nothing";
/// the brief's L7; T9's and P1's shape): C2's cohort and script, two arms from one deterministic
/// start on `KEY_1`; arm A finds the first quiet point in fires 300..=450 (`cohort_quiet`: no
/// cached wait, no queued unload, nothing in flight, no act, re-arm or idle return due within
/// the warm's own horizon, no delivery, batch, restock or Ceres step due, no IC-day boundary, no
/// hull step); arm B runs to it and reboots there, every pilot reconnected, on the same roster;
/// both read `+tv` and the dispatcher's site map, warm (arm A in zero fires; arm B's warm fires
/// write nothing in the mining namespaces and build a site and a home), jump `C3_WINDOW` (320) real
/// seconds and fire once; then both run to fire 600. The post-images are equal by name, the
/// players landed acts after the point in both arms, as many in each, and neither arm paged.
#[test]
fn sm12c_a_restart_inside_the_cohort_changes_nothing() {
    let _lock = shared();
    let a = c3_arm(None);
    let b = c3_arm(Some(a.point));
    println!(
        "cost_lane: C3 point {}; arm B's warm fires {:?}; arm B's post-reboot surface fires' max \
         {} us; players' landings after the point A {} B {}",
        a.point, b.warm, b.after_us, a.played_after, b.played_after
    );
    // The twin compares two worlds the players moved in after the point (the review's tests/T2),
    // never two idle ones.
    assert!(a.played_after > 0, "the players played after the point");
    assert_eq!(
        a.played_after, b.played_after,
        "as many player landings after the point in both arms"
    );
    assert!(
        b.warm.iter().any(|(_, word, us)| word == "Site" && *us > 0),
        "arm B's warm builds a site: {:?}",
        b.warm
    );
    assert!(
        b.warm.iter().any(|(_, word, _)| word == "Home"),
        "arm B's warm builds a home: {:?}",
        b.warm
    );
    let first = a
        .image
        .iter()
        .find(|(name, value)| b.image.get(*name) != Some(*value))
        .map(|(name, value)| (name.clone(), value.clone(), b.image.get(name).cloned()));
    assert!(
        first.is_none() && a.image.len() == b.image.len(),
        "the restart changed {first:?} (likelier RAM causes: stock_attempts, drip_refused, \
         emergency_checked, relet_attempts, the mill and dispatch back-offs)"
    );
}

// ---------------------------------------------------------------------------
// C6 — the month (`#[ignore]`d, release only, `scripts/cost-lane.sh`)
// ---------------------------------------------------------------------------

/// The five Ceres sites, in the lane's site order, with their venue words.
const CERES_SITES: [(&str, &str); 5] = [
    ("commons_port", "commons"),
    ("kerwan_port", "kerwan"),
    ("ahuna_port", "ahuna"),
    ("juling_port", "juling"),
    ("occator_port", "occator"),
];
/// The Vesta site.
const VESTA: (&str, &str) = ("rheasilvia_port", "rheasilvia");
/// The mirror's margin: the month STOPs at this many applied markers (O-27).
const MIRROR_MARGIN: usize = 3_900;
/// The Flagged Markers line's threshold (three quarters of 4,096): a figure, never a halt.
const MIRROR_FLAG: usize = 3_072;
/// The largest frame a feed may send, bytes (asserted always).
const MAX_FRAME_BYTES: usize = 256 * 1_024;
/// Real seconds in an IC day at rate 24.
const DAY_FIRES: u64 = 3_600;

/// A month knob (the harness's rule): unset reads `default`; a value that does not parse or is
/// out of range panics naming the knob.
fn knob(name: &str, default: u32, ok: impl Fn(u32) -> bool) -> u32 {
    match std::env::var(name) {
        Err(_) => default,
        Ok(text) => {
            let n: u32 = text
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("{name}={text:?} is not a number"));
            assert!(ok(n), "{name}={n} is out of range");
            n
        }
    }
}

/// Resident memory, kibibytes (`harness.rs`'s reader).
fn resident_kib() -> Option<u64> {
    memory_stats::memory_stats()
        .map(|stats| u64::try_from(stats.physical_mem).unwrap_or(u64::MAX) / 1_024)
}

/// The world's money: every `tacsim.credit` account's posted balance, summed
/// (`relay_registry.rs`'s `money_total`).
/// `who`'s credit accounts' posted balance and live holds, summed (minor units): a stuck
/// pilot's figure.
fn funds(tw: &TestWorld, who: ObjId) -> (i128, i128) {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .map_or((0, 0), |economy| {
            economy
                .snapshot()
                .accounts()
                .filter(|a| {
                    a.instrument.as_str() == "tacsim.credit"
                        && a.owner == mush_org::PrincipalId::Object(who)
                })
                .fold((0, 0), |(posted, held), a| {
                    (
                        posted + a.posted_balance.widen(),
                        held + a.held_amount.widen(),
                    )
                })
        })
}

fn money_total(tw: &TestWorld) -> i128 {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .map_or(0, |economy| {
            economy
                .snapshot()
                .accounts()
                .filter(|a| a.instrument.as_str() == "tacsim.credit")
                .map(|a| a.posted_balance.widen())
                .sum()
        })
}

/// A feed frame through the feed's own query (`quarry.rs`'s `ground`, `drones.rs`'s
/// `map_frame`): its JSON and its time, µs.
fn feed_frame(rig: &mut Rig, who: ObjId, name: &str, args: Value) -> (Value, u64) {
    let started = Instant::now();
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
    let frame = out
        .events()
        .iter()
        .find_map(|(_, event)| match event {
            OutputEvent::Reply { event, .. } => match event.as_ref() {
                OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(Value::Null);
    (frame, micros(started))
}

/// **The month's scripted tenure** (doc 20a §5.3.9; the brief's L11.4): one renewal, one contest
/// and one unpaid-fee lapse, each driven by its pilots between fires and asserted seen.
#[derive(Clone, Debug, Default)]
struct Script {
    /// The contest's sounder (A) and staker (B), pilot indexes, and the block.
    contest: Option<(usize, usize)>,
    block: Option<char>,
    /// 0 waiting for A's loud sounding; 1 B to the block; 2 B scans; 3 B stakes; 4 A contests;
    /// 5 done.
    step: u8,
    /// The renewal's pilot index and whether it renewed.
    renewer: Option<usize>,
    renewed: bool,
    /// The fee-lapse pilot index and whether its fee was withdrawn (the cancel confirmed).
    lapser: Option<usize>,
    withdrawn: bool,
    /// The fees the lapser tried to withdraw.
    tried: BTreeSet<u64>,
    /// Why a scripted act could not be made, if one could not.
    notes: Vec<String>,
}

impl Script {
    /// Drive the script one decision (after the cohort's own step).
    #[allow(clippy::too_many_lines)]
    fn drive(&mut self, rig: &mut Rig, cohort: &mut Cohort) {
        let fire = cohort.fire;
        // The fee lapse: the lapser's first claim, its fee withdrawn after landing — counted
        // withdrawn only when the economy confirms the cancel (the review's correctness/F3); a fee
        // already accepted refuses it, and the lapser tries its next claim's fee instead.
        if let Some(i) = self.lapser
            && !self.withdrawn
            && let Some(letter) = cohort.pilots[i].claims.first().copied()
        {
            let p = &cohort.pilots[i];
            if let Some(fee) = block_claim(&mut rig.tw, p.site, letter).and_then(|claim| claim.fee)
                && self.tried.insert(fee)
            {
                let out = run(&mut rig.tw, p.who, &format!("@contract/cancel #{fee}"));
                println!("cost_lane: month: pilot {i} withdraws its fee on {letter}: {out}");
                if out.contains(&format!("Contract #{fee} is cancelled.")) {
                    self.withdrawn = true;
                } else {
                    self.notes.push(format!(
                        "the fee #{fee} on {letter} was not withdrawn: {out}"
                    ));
                }
            }
        }
        // The renewal: the renewer's first claim past the worked floor.
        if let Some(i) = self.renewer
            && !self.renewed
        {
            let p = cohort.pilots[i].clone();
            if let Some(letter) = p.claims.first().copied()
                && let Some(claim) = block_claim(&mut rig.tw, p.site, letter)
                && claim.lapsed_ic.is_none()
            {
                let worked = worked_now(&mut rig.tw, p.site, letter)
                    .saturating_sub(claim.worked_at_term_start);
                if worked >= tacsim::surface::tenure::WORKED_FLOOR_T {
                    let line = format!("+claim/renew {letter} go");
                    let (out, n) =
                        timed_command(rig, &mut cohort.tally, (p.who, "claim/renew"), &line);
                    println!("cost_lane: month: pilot {i} renews {letter} at {worked} t: {out}");
                    if n > 0 {
                        self.renewed = true;
                    }
                }
            }
        }
        // The contest.
        let Some((a, b)) = self.contest else {
            return;
        };
        let on_pad = |p: &Pilot| {
            matches!(
                p.phase,
                Phase::HandScan | Phase::HandDrive | Phase::HandMine | Phase::HandLaunch
            )
        };
        match self.step {
            0 => {
                let pa = &cohort.pilots[a];
                if let Some(block) = pa.loud_block
                    && block_claim(&mut rig.tw, pa.site, block)
                        .is_none_or(|claim| claim.lapsed_ic.is_some())
                {
                    self.block = Some(block);
                    println!(
                        "cost_lane: month: the contest's block {block} (pilot {a}'s loud sounding)"
                    );
                    self.step = 1;
                }
            }
            1..=3 => {
                let Some(letter) = self.block else {
                    return;
                };
                let pb = cohort.pilots[b].clone();
                if !on_pad(&pb) || pb.site != cohort.pilots[a].site {
                    return;
                }
                let tv = pb.rigs[0];
                if in_bay(&mut rig.tw, tv) || !rig_idle(&mut rig.tw, tv) {
                    return;
                }
                cohort.pilots[b].retry_at = fire + 600;
                let (line, verb) = match self.step {
                    1 => (format!("+tv/1 drive {letter} go"), "tv/drive"),
                    2 => ("+tv/1 scan go".to_owned(), "tv/scan"),
                    _ => ("+claim/stake go".to_owned(), "claim/stake"),
                };
                let (out, n) = timed_command(rig, &mut cohort.tally, (pb.who, verb), &line);
                println!(
                    "cost_lane: month: contest step {}: pilot {b} {line}: {out}",
                    self.step
                );
                if n > 0 || self.step == 2 {
                    if self.step == 3 {
                        cohort.pilots[b].claims.push(letter);
                    }
                    self.step += 1;
                } else if self.step == 3 {
                    self.notes
                        .push(format!("the contest's stake was refused: {out}"));
                    self.step = 5;
                }
            }
            4 => {
                let Some(letter) = self.block else {
                    return;
                };
                let pa = cohort.pilots[a].clone();
                if docked_at(&rig.tw, ObjId(pa.row.hull)).as_deref() != Some(pa.site) {
                    return;
                }
                let line = format!("+claim/contest {letter} go");
                let (out, n) =
                    timed_command(rig, &mut cohort.tally, (pa.who, "claim/contest"), &line);
                println!("cost_lane: month: pilot {a} contests {letter}: {out}");
                if n == 0 {
                    self.notes.push(format!("the contest was refused: {out}"));
                }
                self.step = 5;
            }
            _ => {}
        }
    }
}

/// **C6 — the cost lane cohort over an IC month** (doc 20 §13's SM.12 row: "the harness cohort,
/// 64 TVs over the sites, every act cycling"; doc 20a §5.3.9; the brief's L11). Release only, by
/// hand: 64 TVs (32 Dray-P hulls of two rigs, a test-only seed variant of the shipped lane that
/// moves only its levers) — two hulls for Rheasilvia Commons (refused at the epoch sky, O-26),
/// the rest round-robin over the five Ceres sites — with Occator's yard and both mills running,
/// for 30 IC days at rate 24 in one-second fires, one reboot at IC day 15. It writes
/// `cost-lane.json` once an IC day and asserts its deterministic invariants always; its timing
/// absolute only under `TACSIM_ASSERT=1`. Knobs: `TACSIM_COHORT_TVS` (even, 2..=64),
/// `TACSIM_COHORT_IC_DAYS` (1..=30), `TACSIM_COST_JSON`.
#[test]
#[ignore = "the cost lane's month: release only, by hand (scripts/cost-lane.sh)"]
#[allow(clippy::too_many_lines, clippy::assertions_on_constants)]
fn sm12c_cost_lane_cohort_over_an_ic_month() {
    let _lock = exclusive();
    assert!(
        !cfg!(debug_assertions),
        "the cost lane runs in release: scripts/cost-lane.sh"
    );
    let wall = Instant::now();
    let tvs = knob("TACSIM_COHORT_TVS", 64, |n| {
        n % 2 == 0 && (2..=64).contains(&n)
    });
    let days = knob("TACSIM_COHORT_IC_DAYS", 30, |n| (1..=30).contains(&n));
    let path = cohort::report_path(
        &std::env::var("TACSIM_COST_JSON").unwrap_or_else(|_| "cost-lane.json".to_owned()),
    );
    let hulls = tvs / 2;
    let vesta = if hulls >= 10 { 2 } else { 0 };
    let levers = Levers::default();
    let lane = cohort_lane(hulls, tvs, &levers);
    // **The levers this run booted**, each against the shipped value (the review's
    // conformance/F8: the month's own variant, at its own hulls and rigs — C7 pins the 64-TV one).
    let levers_diff: BTreeMap<String, (String, String)> = {
        let (a, b): (Value, Value) = (
            serde_json::from_str(LANE_JSON).expect("JSON"),
            serde_json::from_str(&lane).expect("JSON"),
        );
        cohort::json_diff(&a, &b)
            .into_iter()
            .filter(|path| path != "manifest/digest")
            .map(|path| {
                let pair = (at(&a, &path), at(&b, &path));
                (path, pair)
            })
            .collect()
    };
    for (path, (shipped, run)) in &levers_diff {
        println!("cost_lane: month lever {path}: shipped {shipped}, this run {run}");
    }
    let mut report = cohort::Report {
        schema: 1,
        ..cohort::Report::default()
    };
    report.cohort = json!({
        "tvs": tvs,
        "hulls": hulls,
        "active_tvs": (hulls - vesta) * 2 + 2,
        "vesta_hulls": vesta,
        "sites": SITES,
        "ic_days": days,
        "levers": format!("{levers:?}"),
        "levers_diff": levers_diff,
        "rate": RATE,
        "ephemeris_rate": 0,
        "vesta_leg_ic_s": 3_127_945,
    });
    println!(
        "cost_lane: month TVs {tvs} (active {} with the yard's 2), hulls {hulls}, Vesta {vesta}, \
         IC days {days}, levers {levers:?}; the Ceres-Vesta leg 3,127,945 IC s (36.2 IC days)",
        (hulls - vesta) * 2 + 2
    );
    // The world.
    let booted = Instant::now();
    let mut rig = stocked(Roster::seeded(lane));
    assert_lane(&rig);
    for entry in SITES {
        open_site(&mut rig, entry);
    }
    // The cohort: hull i's pilot, kitted up at Ceres (enrolled, refitted, licensed, two leases).
    let t0 = rig.tw.world().now();
    let mut cohort = Cohort::default();
    let mut kit_errors = Vec::new();
    for i in 0..usize::try_from(hulls).unwrap_or(0) {
        let vesta_hull = i >= usize::try_from(hulls - vesta).unwrap_or(0);
        let (site, venue) = if vesta_hull {
            VESTA
        } else {
            CERES_SITES[i % CERES_SITES.len()]
        };
        let who = format!("Pilot{i:02}");
        let name = format!("Run {i:02}");
        let seed = i64::try_from(i).unwrap_or(0) + 100;
        rig.variant = "the month".to_owned();
        let pilot = pilot(&mut rig, &who, seed);
        let out = run(&mut rig.tw, pilot, &format!("+coop/enrol {name}"));
        let Some(charter) = tacsim::port::live_charter(rig.tw.world(), pilot) else {
            kit_errors.push(format!("pilot {i}: {out}"));
            continue;
        };
        let row = port_map(&rig.tw, "ceres_port")
            .hulls
            .into_iter()
            .find(|row| row.hull == charter.hull.0)
            .expect("the hull's row");
        let out = run(&mut rig.tw, pilot, "+coop/refit prospector");
        assert!(out.contains("as a Dray-P"), "pilot {i}: {out}");
        let (out, _) = normal_cmd(&mut rig, pilot, "+licence/buy go", "licence.buy");
        assert!(out.contains("licenses you to prospect"), "pilot {i}: {out}");
        let _ = lease_one(&mut rig, pilot);
        let _ = lease_one(&mut rig, pilot);
        let rigs = rigs_of(&rig.tw, pilot);
        let mode = if i % 4 == 3 { Mode::Hand } else { Mode::Plan };
        cohort.pilots.push(Pilot {
            index: i,
            who: pilot,
            row,
            rigs,
            site,
            venue,
            mode,
            family: if i % 2 == 0 { "metal_ore" } else { "ice_ore" },
            phase: Phase::Bunker,
            heard: String::new(),
            targets: Vec::new(),
            claims: Vec::new(),
            loops: 0,
            since_ic: ic_now(&rig.tw),
            start_at: t0 + i64::try_from(i).unwrap_or(0) * 37,
            rounds: 0,
            sound_pending: false,
            stay_on_claim: false,
            loud_block: None,
            retry_at: 0,
            bunker_rounds: 0,
            filed: false,
            dry: false,
            dry_sites: BTreeSet::new(),
            dry_families: BTreeSet::new(),
            filed_family: None,
            progress_ic: ic_now(&rig.tw),
            progress_sig: (0, 0),
        });
    }
    assert!(kit_errors.is_empty(), "the cohort kits up: {kit_errors:?}");
    // The parties nearest the economy's per-party cap (`MAX_LIVE_CONTRACTS_PER_PARTY`): a figure
    // for the owner — a Draw from a vendor already on that many live contracts refuses.
    let mut parties: BTreeMap<String, usize> = BTreeMap::new();
    let names: BTreeMap<u64, String> = port_map(&rig.tw, "ceres_port")
        .orgs
        .iter()
        .map(|(id, row)| (row.org, id.to_string()))
        .collect();
    if let Some(economy) = rig.tw.world().module_state::<mush_economy::EconomyStore>() {
        for contract in economy.snapshot().contracts().filter(|c| c.state.is_live()) {
            for party in &contract.consent.parties {
                let name = match party {
                    mush_org::PrincipalId::Organization(org) => names
                        .get(&org.0)
                        .cloned()
                        .unwrap_or_else(|| format!("{party:?}")),
                    other => format!("{other:?}"),
                };
                *parties.entry(name).or_insert(0) += 1;
            }
        }
    }
    let busiest: Vec<(String, usize)> = {
        let mut all: Vec<(String, usize)> = parties.into_iter().collect();
        all.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        all.into_iter().take(4).collect()
    };
    println!(
        "cost_lane: month live contracts by party after the kit-up (cap {}): {busiest:?}",
        mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY
    );
    report.cohort["live_contracts_busiest"] = json!(busiest);
    // **The engine cap that binds the cohort** (the brief's L10: an engine cap that would block
    // the month STOPs it and is reported): a vendor already on the economy's per-party cap of live
    // contracts refuses every Draw — Occator's propellant, the Co-op's provisions — and the
    // Registry every stake, so the cohort could never bunker or claim.
    let capped: Vec<&(String, usize)> = busiest
        .iter()
        .filter(|(name, live)| {
            ["occator", "coop", "freeport_registry"].contains(&name.as_str())
                && *live >= mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY
        })
        .collect();
    if !capped.is_empty() {
        report.stopped = json!({
            "reason": format!(
                "an engine cap: the economy's MAX_LIVE_CONTRACTS_PER_PARTY ({}) — a vendor \
                 already on that many live contracts refuses every Draw",
                mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY
            ),
            "ic_day": 0,
            "parties": capped,
            "tvs": tvs,
        });
        cohort::write_report(&path, &report);
        panic!(
            "cost_lane: month STOPPED at the kit-up: {capped:?} reach the economy's per-party cap \
             of {} live contracts (mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY), so no pilot can \
             bunker; a smaller TACSIM_COHORT_TVS fits it (an engine cap, the owner's: never \
             raised here)",
            mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY
        );
    }
    rig.own.clear();
    let setup_s = booted.elapsed().as_secs_f64();
    println!(
        "cost_lane: month setup {setup_s:.1} s; {} pilots",
        cohort.pilots.len()
    );
    // The script's pilots: the first site with two hand pilots contests; the second hand pilot
    // of the cohort withdraws its first fee; the third renews.
    let hands: Vec<usize> = cohort
        .pilots
        .iter()
        .filter(|p| p.mode == Mode::Hand && p.site != VESTA.0)
        .map(|p| p.index)
        .collect();
    let mut script = Script::default();
    for (n, a) in hands.iter().enumerate() {
        if let Some(b) = hands[n + 1..]
            .iter()
            .find(|b| cohort.pilots[**b].site == cohort.pilots[*a].site)
        {
            script.contest = Some((*a, *b));
            break;
        }
    }
    script.lapser = hands.iter().copied().find(|i| {
        Some(*i) != script.contest.map(|c| c.0) && Some(*i) != script.contest.map(|c| c.1)
    });
    script.renewer = hands.iter().copied().find(|i| {
        Some(*i) != script.lapser
            && Some(*i) != script.contest.map(|c| c.0)
            && Some(*i) != script.contest.map(|c| c.1)
    });
    if let Some(i) = script.renewer {
        cohort.pilots[i].stay_on_claim = true;
    }
    println!(
        "cost_lane: month script: contest {:?}, fee lapse {:?}, renewal {:?}",
        script.contest, script.lapser, script.renewer
    );
    if script.contest.is_none() {
        script.notes.push(
            "the contest cannot be scripted at this cohort: no Ceres site holds two hand pilots \
             under the brief's rules (pilot i plays by hand when i % 4 == 3; the Ceres-site hulls \
             round-robin over the five sites)"
                .to_owned(),
        );
    }
    // The month.
    assert_eq!(world_rate(&rig.tw), RATE);
    let money0 = money_total(&rig.tw);
    let ic0 = ic_now(&rig.tw);
    let storefronts: Vec<(String, VaultId)> = port_map(&rig.tw, "ceres_port")
        .vendors
        .iter()
        .map(|(org, row)| (org.to_string(), VaultId(row.vault)))
        .collect();
    let store_ore0: BTreeMap<String, u64> = ORES
        .iter()
        .map(|(good, _)| {
            let total = storefronts
                .iter()
                .map(|(_, vault)| pool(&mut rig.tw, *vault, good))
                .sum();
            ((*good).to_owned(), total)
        })
        .collect();
    rig.ops_per_fire.clear();
    let total = u64::from(days) * DAY_FIRES;
    let reboot_at = (days >= 15).then_some(15 * DAY_FIRES);
    let mut span = Span::default();
    let mut day_span = Span::default();
    let mut day_rows: Vec<Value> = Vec::new();
    let mut breaches: Vec<String> = Vec::new();
    let mut mirror_max = 0_usize;
    let (mut mirror_flag_day, mut mirror_full_day) = (None, None);
    let mut ring_max = 0_usize;
    let mut ring_fill: BTreeMap<u32, usize> = BTreeMap::new();
    let mut held_fires: BTreeMap<u32, u64> = BTreeMap::new();
    let mut hold_run: BTreeMap<u32, u64> = BTreeMap::new();
    let mut hold_longest: BTreeMap<u32, u64> = BTreeMap::new();
    let mut min_headroom = usize::MAX;
    let mut clerk_days: BTreeMap<u64, BTreeMap<&'static str, u64>> = BTreeMap::new();
    let mut clerk_idle_due: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut clerk_unlanded = 0_u64;
    let ceres_clerk = port_map(&rig.tw, "ceres_port")
        .clerk
        .map(|clerk| ObjId(clerk.thing));
    let mut back_pressure_lines = 0_u64;
    let mut plan_wait_markers = 0_u64;
    let mut frames: BTreeMap<&'static str, (Vec<u64>, usize)> = BTreeMap::new();
    let mut post_reboot: Option<(u64, bool, u64)> = None;
    let mut reboot_s = 0.0_f64;
    let mut why_markers = 0_u64;
    let mut wait_markers = 0_u64;
    let mut phases: Vec<Phase> = cohort.pilots.iter().map(|p| p.phase.clone()).collect();
    let mut stopped = Value::Null;
    let mut deliverable_zero_days = 0_u64;
    let mut backlog_max = (0_u64, 0_u64);
    let mut f = 0_u64;
    while f < total {
        if Some(f) == reboot_at {
            let started = Instant::now();
            let _ = count_and_fold(&mut rig);
            rig = reboot(rig);
            reboot_s = started.elapsed().as_secs_f64();
            println!("cost_lane: month reboot at IC day 15: {reboot_s:.2} s");
            post_reboot = Some((f, false, 0));
        }
        // The Ceres clerk's own earlier submission still unlanded as the fire starts (near zero:
        // the rig lands every submission between fires).
        let clerk_in_flight = ceres_clerk.is_some_and(|clerk| {
            rig.tw
                .engine()
                .queued_authoritative_actors()
                .any(|actor| actor == clerk)
        });
        // The Ceres clerk's steps due as the fire begins — the state its port fire decides on (the
        // investigation of 2026-10-26: judged after the step, every one of the 177 "idle while a
        // batch was due" fires of the 30-TV month was ore that landed after the clerk had decided,
        // the step's own landings and the pilots' sales, and milled at the next port fire).
        let due_at_decision = clerk_due(&mut rig, (levers.mill_batch, levers.mill_per_day));
        let fired = second(&mut rig);
        span.record(&fired);
        day_span.record(&fired);
        cohort.step(&mut rig, &fired);
        script.drive(&mut rig, &mut cohort);
        if let Some((at, seen, max_us)) = post_reboot.as_mut()
            && f < *at + 150
            && let Some(stats) = fired.surface
        {
            if stats.warmed == Warmed::Site && stats.warm_us > 0 {
                *seen = true;
            }
            *max_us = (*max_us).max(stats.elapsed_us);
        }
        let now_wall = rig.tw.world().now();
        // The rings of every standing queue.
        for p in &cohort.pilots {
            for (slot, tv) in p.rigs.iter().enumerate() {
                let Some(record) = record_of(&mut rig.tw, *tv) else {
                    continue;
                };
                if record.queued.is_none() {
                    hold_run.remove(&tv.0);
                    continue;
                }
                // A live plan's standing queue: its wait as `+tv/<n> plan show` names it (the
                // brief's L11.7 `PlanWait::Markers` count; a pure read, between fires).
                if record
                    .plan
                    .as_ref()
                    .is_some_and(tacsim::surface::plan::FillPlan::live)
                {
                    let shown = run(&mut rig.tw, p.who, &format!("+tv/{} plan show", slot + 1));
                    if shown.contains(tacsim::port::tv::WAIT_MARKERS) {
                        plan_wait_markers += 1;
                    }
                }
                let vault = hopper(&rig.tw, *tv);
                let goods = ORES
                    .iter()
                    .filter(|(good, _)| pool(&mut rig.tw, vault, good) > 0)
                    .count();
                let ring = marker_ring(&mut rig.tw, vault).unwrap_or_default();
                ring_max = ring_max.max(ring.len());
                let fill = ring_fill.entry(tv.0).or_insert(0);
                *fill = (*fill).max(ring.len());
                let headroom = (0..=128)
                    .take_while(|n| ring.admits(*n, now_wall))
                    .last()
                    .unwrap_or(0);
                min_headroom = min_headroom.min(headroom);
                if !ring.admits(goods, now_wall) {
                    *held_fires.entry(tv.0).or_insert(0) += 1;
                    let run = hold_run.entry(tv.0).or_insert(0);
                    *run += 1;
                    let longest = hold_longest.entry(tv.0).or_insert(0);
                    *longest = (*longest).max(*run);
                } else {
                    hold_run.remove(&tv.0);
                }
            }
        }
        // The mirror.
        let markers = rig
            .tw
            .world()
            .module_state::<mush_inventory::InventoryStore>()
            .map_or(0, mush_inventory::InventoryStore::markers);
        let complete = rig
            .tw
            .world()
            .module_state::<mush_inventory::InventoryStore>()
            .is_some_and(mush_inventory::InventoryStore::is_complete);
        mirror_max = mirror_max.max(markers);
        let day = f / DAY_FIRES;
        if markers > MIRROR_FLAG && mirror_flag_day.is_none() {
            mirror_flag_day = Some(day);
        }
        if markers > 4_096 && mirror_full_day.is_none() {
            mirror_full_day = Some(day);
        }
        // The Ceres clerk.
        if fired.port.is_some() {
            match fired.clerk {
                Some(act) => {
                    *clerk_days
                        .entry(day)
                        .or_default()
                        .entry(act.word())
                        .or_insert(0) += 1;
                }
                None => {
                    for step in &due_at_decision {
                        *clerk_idle_due.entry(*step).or_insert(0) += 1;
                    }
                }
            }
            if clerk_in_flight {
                clerk_unlanded += 1;
            }
        }
        // The pilots' streams: the marker lines.
        for (_, event) in &fired.events {
            let text = format!("{event:?}");
            if text.contains(tacsim::port::tv::WHY_MARKERS_QUEUED) {
                why_markers += 1;
            }
            if text.contains("unload log is full") {
                wait_markers += 1;
            }
            if BACK_PRESSURE.iter().any(|line| text.contains(line)) {
                back_pressure_lines += 1;
            }
        }
        // The frames: `site.map` at every arrival and plan filing, `body.ground` once a day.
        for (n, p) in cohort.pilots.iter().enumerate() {
            let arrived = phases[n] == Phase::Outbound && p.phase != Phase::Outbound;
            let filed = phases[n] == Phase::PlanFile && p.phase == Phase::PlanQueue;
            if arrived || filed {
                let (frame, us) =
                    feed_frame(&mut rig, p.who, "site.map", json!({ "view": "site" }));
                let bytes = frame.to_string().len();
                let entry = frames.entry("site.map").or_insert((Vec::new(), 0));
                entry.0.push(us);
                entry.1 = entry.1.max(bytes);
            }
            phases[n] = p.phase.clone();
        }
        // The day's end: the day row and the report, written before any assertion.
        if (f + 1).is_multiple_of(DAY_FIRES) {
            let whos: Vec<ObjId> = cohort.pilots.iter().map(|p| p.who).collect();
            for who in whos {
                let (frame, us) =
                    feed_frame(&mut rig, who, "body.ground", json!({ "body": "ceres" }));
                let bytes = frame.to_string().len();
                let entry = frames.entry("body.ground").or_insert((Vec::new(), 0));
                entry.0.push(us);
                entry.1 = entry.1.max(bytes);
            }
            let yard_hold = VaultId(yard(&rig.tw).hold.expect("the yard hold"));
            let held = pool(&mut rig.tw, yard_hold, "ice_ore");
            let occ = storefront(&rig.tw, "occator");
            let front_ice = pool(&mut rig.tw, occ, "ice_ore");
            let exch = storefront(&rig.tw, "freeport_exchange");
            let front_metal = pool(&mut rig.tw, exch, "metal_ore");
            backlog_max = (backlog_max.0.max(front_ice), backlog_max.1.max(front_metal));
            if held > 0
                && tacsim::port::yard::deliverable(held, front_ice, levers.mill_per_day) == 0
            {
                deliverable_zero_days += 1;
            }
            let loops: u32 = cohort.pilots.iter().map(|p| p.loops).sum();
            // The mirror's composition by the rings that retain its markers (each transfer marker sits
            // in its source endpoint's ring): which rings hold
            // it (decision (c) relieves only the hoppers').
            let hoppers: BTreeSet<u64> = cohort
                .pilots
                .iter()
                .flat_map(|p| p.rigs.clone())
                .chain(yard_rigs(&rig.tw))
                .map(|tv| hopper(&rig.tw, tv).0)
                .collect();
            let holds: BTreeSet<u64> = cohort
                .pilots
                .iter()
                .map(|p| hold_vault(&rig.tw, &p.row).0)
                .chain(std::iter::once(yard_hold.0))
                .collect();
            let fronts: BTreeSet<u64> = storefronts.iter().map(|(_, vault)| vault.0).collect();
            let mut by_source: BTreeMap<&'static str, usize> = BTreeMap::new();
            for (class, vaults) in [
                ("hopper", &hoppers),
                ("hold", &holds),
                ("storefront", &fronts),
            ] {
                let held: usize = vaults
                    .iter()
                    .map(|vault| {
                        marker_ring(&mut rig.tw, VaultId(*vault)).map_or(0, |ring| ring.len())
                    })
                    .sum();
                by_source.insert(class, held);
            }
            let ringed: usize = by_source.values().sum();
            by_source.insert("elsewhere", markers.saturating_sub(ringed));
            let stuck: Vec<String> = cohort
                .pilots
                .iter()
                .filter(|p| {
                    !matches!(p.phase, Phase::Refused | Phase::Parked)
                        && ic_now(&rig.tw) - p.progress_ic >= DAY
                })
                .map(|p| format!("pilot {} {:?}", p.index, p.phase))
                .collect();
            day_rows.push(json!({
                "day": day,
                "surface_p99_us": cohort::p99(&day_span.surface_us),
                "surface_max_us": cohort::max(&day_span.surface_us),
                "port_p99_us": cohort::p99(&day_span.port_us),
                "markers": markers,
                "markers_by_source": by_source,
                "mirror_max": mirror_max,
                "ring_max": ring_max,
                "markers_held": day_span.markers_held,
                "clerk": clerk_days.get(&day).cloned().unwrap_or_default(),
                "loops": loops,
                "stuck": stuck,
                "yard_hold_ice": held,
                "storefront_ice": front_ice,
                "storefront_metal": front_metal,
                "ledger": rig.ledger.len(),
                "rss_kib": resident_kib(),
                "wall_s": wall.elapsed().as_secs_f64(),
            }));
            println!("cost_lane: month day {day}: markers by source {by_source:?}");
            println!(
                "cost_lane: month day {day}: surface p99 {} us, markers {markers}, ring max \
                 {ring_max}, loops {loops}, stuck {}, wall {:.0} s",
                cohort::p99(&day_span.surface_us),
                day_rows
                    .last()
                    .map_or(0, |row| row["stuck"].as_array().map_or(0, Vec::len)),
                wall.elapsed().as_secs_f64()
            );
            day_span = Span::default();
            report.days = Value::Array(day_rows.clone());
            month_report(
                &mut report,
                (&rig, &cohort, &span),
                &json!({
                    "mirror_max": mirror_max, "flag_day": mirror_flag_day,
                    "full_day": mirror_full_day, "complete": complete,
                    "marker_pages": tacsim::port_tick::marker_pages(rig.tw.world()),
                }),
                &json!({
                    "ring_max": ring_max, "min_headroom": min_headroom,
                    "held_fires": held_fires.clone(), "longest_hold_fires": hold_longest.clone(),
                    "why_markers_lines": why_markers, "unload_log_full_lines": wait_markers,
                    "plan_wait_markers_shown": plan_wait_markers,
                    "unload_markers_refusals": rig.tally.unload_markers,
                    "markers_full_finishes": rig.tally.markers_full,
                    "back_pressure": back_pressure_lines + cohort.tally.back_pressure,
                    "fills": ring_fill.clone(),
                }),
                wall.elapsed().as_secs_f64(),
            );
            report.clerk = json!({
                "by_day": clerk_days,
                "idle_while_due_by_step": clerk_idle_due,
                "own_submission_unlanded_fires": clerk_unlanded,
                "stock_arm_five_clerks_deferred_max": span.stock_max[2],
                "note": "a port fire in which the Ceres clerk acted on nothing while a step was \
                         due as the fire began (the state it decided on), counted once for each \
                         step due (batch, restock, list, drip; re-pinned from stock::stock_step's \
                         order)",
            });
            report.frames = json!(
                frames
                    .iter()
                    .map(|(name, (us, bytes))| (
                        (*name).to_owned(),
                        json!({
                            "p50_us": cohort::p50(us), "p99_us": cohort::p99(us),
                            "max_us": cohort::max(us), "bytes_max": bytes, "n": us.len()
                        })
                    ))
                    .collect::<BTreeMap<String, Value>>()
            );
            report.breaches = json!(breaches.clone());
            cohort::write_report(&path, &report);
        }
        if !complete || markers >= MIRROR_MARGIN {
            stopped = json!({ "reason": if complete { "the applied-marker mirror is within the margin" } else { "the applied-marker mirror is incomplete" }, "ic_day": day, "markers": markers });
            report.stopped = stopped.clone();
            cohort::write_report(&path, &report);
            panic!("cost_lane: month STOPPED at IC day {day}: {stopped} (O-27)");
        }
        f += 1;
    }
    // The month's end.
    assert_eq!(world_rate(&rig.tw), RATE);
    let ic_span = ic_now(&rig.tw) - ic0;
    println!(
        "cost_lane: month IC span {ic_span} s ({} IC days)",
        ic_span / DAY
    );
    span.print("month", &rig.ops_per_fire);
    print_tally(&rig, "month");
    for (verb, samples) in &cohort.tally.timed {
        println!(
            "cost_lane: month command {verb} p50 {} p99 {} max {} us (n {})",
            cohort::p50(samples),
            cohort::p99(samples),
            cohort::max(samples),
            samples.len()
        );
    }
    let store_ore1: BTreeMap<String, u64> = ORES
        .iter()
        .map(|(good, _)| {
            let total = storefronts
                .iter()
                .map(|(_, vault)| pool(&mut rig.tw, *vault, good))
                .sum();
            ((*good).to_owned(), total)
        })
        .collect();
    let milled: Vec<Value> = [("freeport_exchange", "metal_ore"), ("occator", "ice_ore")]
        .iter()
        .map(|(org, ore)| json!({ "org": org, "ledger": format!("{:?}", mill_ledger(&mut rig.tw, org, ore)) }))
        .collect();
    let mined: u64 = SITES
        .iter()
        .flat_map(|site| {
            tacsim::port::def::BLOCK_LETTERS
                .iter()
                .map(move |letter| (*site, *letter))
        })
        .map(|(site, letter)| worked_now(&mut rig.tw, site, letter))
        .sum();
    let dispatch_family = handles(&rig.tw).families.dispatch;
    let delivered = typed(
        &mut rig.tw,
        &dispatch_family,
        &tacsim::port::yard::DispatchKey { site: cid(OCCATOR) },
    );
    let loops: Vec<u32> = cohort
        .pilots
        .iter()
        .filter(|p| p.site != VESTA.0)
        .map(|p| p.loops)
        .collect();
    report.activity = json!({
        "loops": cohort.pilots.iter().map(|p| (p.index, p.loops, format!("{:?}", p.phase), p.site)).collect::<Vec<_>>(),
        "loops_min": loops.iter().min(), "loops_max": loops.iter().max(),
        "loops_median": cohort::p50(&loops.iter().map(|n| u64::from(*n)).collect::<Vec<_>>()),
        "mined_t": mined,
        "sold_by_hand_t": cohort.tally.sold,
        "storefront_ore_start": store_ore0, "storefront_ore_end": store_ore1,
        "mills": milled,
        "dispatch": format!("{delivered:?}"),
        "deliverable_zero_days": deliverable_zero_days,
        "storefront_backlog_max": { "ice": backlog_max.0, "metal": backlog_max.1 },
        "vesta": cohort.tally.vesta,
        "longest_phase": cohort.tally.longest.iter().map(|(i, (s, phase))| (i, s, phase.clone())).collect::<Vec<_>>(),
        "staff_bunkers": cohort.tally.staff_bunkers,
        "worked_out": cohort.worked_out.iter().map(|(site, fire)| json!({
            "site": site, "fire": fire, "ic_day": fire / DAY_FIRES,
        })).collect::<Vec<_>>(),
        "parked": cohort.parked.iter().map(|(pilot, fire)| json!({
            "pilot": pilot, "fire": fire, "ic_day": fire / DAY_FIRES,
        })).collect::<Vec<_>>(),
        "rss_kib": resident_kib(),
    });
    println!(
        "cost_lane: month sites come home dry {:?}; pilots parked {:?} (fire, IC day = fire / {})",
        cohort.worked_out, cohort.parked, DAY_FIRES
    );
    report.claims = json!({
        "skips": cohort.tally.skips,
        "renewed": script.renewed,
        "contest_step": script.step,
        "withdrawn": script.withdrawn,
        "notes": script.notes,
    });
    let post = post_reboot.map(|(_, seen, max_us)| (seen, max_us));
    report.decisions = json!({
        "a_off_loop_runner": {
            "site_warm_max_us": span.warmed.get("Site").map(|(_, us)| us),
            "home_warm_max_us": span.warmed.get("Home").map(|(_, us)| us),
            "post_reboot": post,
            "threshold_us": 20_000,
        },
        "b_memo": {
            "tick_per_leg_p99_us": cohort::p99(&span.per_leg),
            "tick_per_leg_max_us": cohort::max(&span.per_leg),
            "hand_drive_p99_us": cohort.tally.timed.get("tv/drive").map(|s| cohort::p99(s)),
            "threshold_us": 10_000,
        },
        "c_ring_relief": {
            "markers_held": span.markers_held, "min_headroom": min_headroom,
            "mirror_max": mirror_max, "mirror_flag_day": mirror_flag_day,
            "why_markers_lines": why_markers, "plan_wait_markers_shown": plan_wait_markers,
            "unload_markers_refusals": rig.tally.unload_markers,
            "markers_full_finishes": rig.tally.markers_full,
        },
        "peek_per_peek_p99_us": cohort::p99(&span.per_peek),
        "reboot_s": reboot_s,
        "setup_s": setup_s,
    });
    month_report(
        &mut report,
        (&rig, &cohort, &span),
        &json!({
            "mirror_max": mirror_max, "flag_day": mirror_flag_day, "full_day": mirror_full_day,
            "marker_pages": tacsim::port_tick::marker_pages(rig.tw.world()),
            "industry_complete": rig.tw.world().module_state::<mush_industry::IndustryStore>().is_some_and(mush_industry::IndustryStore::is_complete),
        }),
        &json!({
            "ring_max": ring_max, "min_headroom": min_headroom, "held_fires": held_fires,
            "longest_hold_fires": hold_longest, "why_markers_lines": why_markers,
            "unload_log_full_lines": wait_markers,
            "plan_wait_markers_shown": plan_wait_markers,
            "unload_markers_refusals": rig.tally.unload_markers,
            "markers_full_finishes": rig.tally.markers_full,
            "back_pressure": back_pressure_lines + cohort.tally.back_pressure,
            "fills": ring_fill,
        }),
        wall.elapsed().as_secs_f64(),
    );
    // The always-asserted invariants, collected (the report is written first).
    let gate_breaches: Vec<String> = rig
        .tally
        .kinds
        .iter()
        .filter_map(|(kind, row)| {
            cohort::gate(kind)
                .filter(|gate| row.max > gate.ceiling)
                .map(|gate| format!("{kind}: {} over {}", row.max, gate.ceiling))
        })
        .collect();
    breaches.extend(gate_breaches);
    let pairs = rig.tally.pairs();
    let unclassified = cohort::unclassified(pairs.iter().map(|(k, a)| (k.as_str(), a.as_str())));
    if !unclassified.is_empty() {
        breaches.push(format!("unclassified {unclassified:?}"));
    }
    if reboot_at.is_some() && !post.is_some_and(|(seen, _)| seen) {
        breaches.push("no Warmed::Site fire in the IC hour after the reboot".to_owned());
    }
    let money1 = money_total(&rig.tw);
    if money1 != money0 {
        breaches.push(format!("the world's money moved: {money0} -> {money1}"));
    }
    let quarantined = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .map_or(0, |store| store.quarantined().len());
    if quarantined > 0 {
        breaches.push(format!("{quarantined} deposits quarantined"));
    }
    for (name, (_, bytes)) in &frames {
        if *bytes >= MAX_FRAME_BYTES {
            breaches.push(format!("{name} frame {bytes} bytes"));
        }
    }
    // Every page of both boots (the reboot folds the first boot's RAM counters into the rig: the
    // review's correctness/F2), the marker headroom's excepted (a figure).
    let paged = pages(&rig);
    if !paged.is_empty() {
        breaches.push(format!("a page beyond the marker headroom's: {paged:?}"));
    }
    // The span is the month's IC days exactly: every fire of it a measured one-second fire (the
    // review's correctness/F1).
    if ic_span != i64::from(days) * DAY {
        breaches.push(format!(
            "the month's IC span {ic_span} s is not {days} IC days"
        ));
    }
    if !rig.tw.world().refused_namespace_writes().is_empty() {
        breaches.push("refused namespace writes".to_owned());
    }
    for kind in MONTH_KINDS {
        if !rig.tally.kinds.contains_key(*kind) {
            breaches.push(format!("{kind} never landed in the month"));
        }
    }
    for gate in WRITE_GATES {
        if !rig.tally.kinds.contains_key(gate.kind) {
            println!("cost_lane: month {} census only", gate.kind);
        }
    }
    if days >= 20 {
        // Each scripted act seen by its landing, never by the command that asked for it (the
        // review's correctness/F3): the renewal's `claim.renew`, the clerk's `claim.unpaid` after
        // the confirmed withdrawal.
        let landed = |kind: &str| rig.tally.kinds.get(kind).is_some_and(|row| row.landed > 0);
        for (name, seen) in [
            (
                "the scripted renewal",
                script.renewed && landed("claim.renew"),
            ),
            (
                "the scripted contest",
                script.contest.is_none() || (script.step == 5 && landed("claim.contest")),
            ),
            (
                "the scripted fee lapse",
                script.withdrawn && landed("claim.unpaid"),
            ),
        ] {
            if !seen {
                breaches.push(format!("{name} not seen: {:?}", script.notes));
            }
        }
    }
    if days >= 10 {
        for p in cohort.pilots.iter().filter(|p| p.site != VESTA.0) {
            if p.loops < 2 {
                breaches.push(format!("pilot {} completed {} loops", p.index, p.loops));
            }
        }
    }
    // Stuck: a Ceres-site pilot whose step has not advanced for an IC day, waits included
    // (`Pilot::progress_ic`: a phase change, or a queued plan's cursor, cycles, trips or sailing
    // plan moving). A parked pilot is not waiting on the world: every Ceres site came home dry for
    // it, which is the figure reported beside (`activity.parked`, `activity.worked_out`).
    let stuck: Vec<String> = cohort
        .pilots
        .iter()
        .filter(|p| {
            p.site != VESTA.0 && p.phase != Phase::Parked && ic_now(&rig.tw) - p.progress_ic >= DAY
        })
        .map(|p| format!("pilot {} {:?}", p.index, p.phase))
        .collect();
    if !stuck.is_empty() {
        breaches.push(format!("stuck pilots {stuck:?}"));
    }
    let p99 = cohort::p99(&span.surface_us);
    if asserting() && p99 >= 5_000 {
        breaches.push(format!(
            "the month's surface-fire p99 {p99} us (TACSIM_ASSERT=1)"
        ));
    }
    report.breaches = json!(breaches.clone());
    report.stopped = stopped;
    report.wall_s = json!(wall.elapsed().as_secs_f64());
    cohort::write_report(&path, &report);
    println!(
        "cost_lane: month wall {:.0} s; report {}; breaches {breaches:?}",
        wall.elapsed().as_secs_f64(),
        path.display()
    );
    assert!(breaches.is_empty(), "the month's breaches: {breaches:?}");
}

/// The gated kinds every month lands (the rest are the census's only).
const MONTH_KINDS: &[&str] = &[
    "start.launch",
    "done.launch",
    "start.scan",
    "done.scan",
    "start.sound",
    "done.sound",
    "start.drive",
    "done.drive",
    "start.mine",
    "done.cycle",
    "start.recover",
    "done.recover",
    "start.unload.queued",
    "done.unload",
    "start.plan_file",
    "start.plan_end",
    "start.drive.plan",
    "start.sound.plan",
    "start.mine.plan",
    "start.recover.plan",
    "claim.stake",
    "claim.seed",
    "site.open",
    "site.worked",
    "site.house_licence",
    "licence.buy",
    "lease.take",
    "mill.batch",
    "mill.restock",
    "dispatch.deliver",
    "dispatch.survey",
];

/// Fill the month report's figure sections from the rig, the cohort and the span.
fn month_report(
    report: &mut cohort::Report,
    (rig, cohort, span): (&Rig, &Cohort, &Span),
    mirror: &Value,
    rings: &Value,
    wall_s: f64,
) {
    let fig = |samples: &[u64]| {
        json!({
            "p50_us": cohort::p50(samples), "p99_us": cohort::p99(samples),
            "max_us": cohort::max(samples), "n": samples.len(),
            "windows_p99_us": cohort::windows(samples, 5),
        })
    };
    report.surface = json!({
        "fire": fig(&span.surface_us),
        "warms": span.warmed.iter().map(|(k, (n, us))| (k.clone(), json!({"n": n, "max_us": us}))).collect::<BTreeMap<String, Value>>(),
        "searches": span.searches, "search_us": span.search_us, "per_leg": fig(&span.per_leg),
        "peeks": span.peeks, "peek_us": span.peek_us, "per_peek": fig(&span.per_peek),
        "markers_held": span.markers_held,
        "max_visited_worked_passed_over": span.surface_max,
        "zero_elapsed": span.zero_elapsed,
    });
    report.port = json!({
        "fire": fig(&span.port_us),
        "max_total": span.port_max, "max_stock_arm_five_clerks": span.stock_max,
        "max_site": span.site_max, "max_dispatch": span.dispatch_max,
        "note": "the rig lands every submission between fires, so Pending deferrals are near zero by construction",
        "queued_max": span.queued_max, "deferrals": span.deferrals,
        "ops_per_fire_p99": cohort::p99(&rig.ops_per_fire), "ops_per_fire_max": cohort::max(&rig.ops_per_fire),
    });
    report.other_ticks = fig(&span.other_us);
    report.writes = json!(
        rig.tally
            .kinds
            .iter()
            .map(|(kind, row)| {
                (
                    kind.clone(),
                    json!({
                        "landed": row.landed, "house": row.house, "clerk": row.clerk,
                        "player": row.player, "max": row.max, "refused": row.refused,
                        "ceiling": cohort::gate(kind).map(|g| g.ceiling),
                        "basis": cohort::gate(kind).map(|g| g.basis.word()),
                    }),
                )
            })
            .chain(
                rig.tally
                    .dropped
                    .iter()
                    .map(|(owner, n)| (owner.clone(), json!({ "dropped": n }))),
            )
            .collect::<BTreeMap<String, Value>>()
    );
    report.landings = json!(
        rig.tally
            .kinds
            .iter()
            .map(|(kind, row)| (
                kind.clone(),
                json!({
                    "p50_us": cohort::p50(&row.land_us), "p99_us": cohort::p99(&row.land_us),
                    "max_us": cohort::max(&row.land_us),
                })
            ))
            .collect::<BTreeMap<String, Value>>()
    );
    let counters = tacsim::surface::site_cache_counters(rig.tw.world());
    report.churn = json!({
        "entries": counters.entries, "builds": counters.builds, "evictions": counters.evictions,
        "site_warm_max_us": span.warmed.get("Site").map(|(_, us)| us),
        "home_warm_max_us": span.warmed.get("Home").map(|(_, us)| us),
    });
    report.planning = json!({
        "tick_per_leg": fig(&span.per_leg),
        "hand_drive": cohort.tally.timed.get("tv/drive").map(|s| fig(s)),
        "plan_drive_landing": rig.tally.kinds.get("start.drive.plan").map(|row| fig(&row.land_us)),
        // The offer's re-search (SM.11's double A*), as an upper bound (the review's
        // conformance/F3): every landed plan drive is a leg the tick searched and the offer's
        // `plan_variants` searched again at landing; beside it the tick's own searches (the
        // house's included), which a count of double searches can never exceed.
        "plan_drive_landings_searched_twice_upper_bound": rig.tally.kinds.get("start.drive.plan").map(|row| row.landed),
        "tick_searches": span.searches,
        "ship_plan": cohort.tally.timed.get("ship/plan").map(|s| fig(s)),
        "ship_undock": cohort.tally.timed.get("ship/undock").map(|s| fig(s)),
        "ship_lift": cohort.tally.timed.get("ship/lift").map(|s| fig(s)),
    });
    report.mirror = mirror.clone();
    report.rings = rings.clone();
    report.wall_s = json!(wall_s);
}

/// **A plan pilot with no plan to file goes home**: rig 1 recovered if it is out, then the
/// captain's queue awaits the rigs, departs for Ceres and sells whatever the hold carries.
fn go_home(rig: &mut Rig, tally: &mut CohortTally, p: &Pilot) {
    if !in_bay(&mut rig.tw, p.rigs[0]) {
        let _ = timed_command(rig, tally, (p.who, "tv/recover"), "+tv/1 recover go");
    }
    for line in [
        "+ship/order await tv all",
        "+ship/order depart ceres",
        "+ship/order sell metal_ore all",
        "+ship/order sell ice_ore all",
    ] {
        let _ = timed_command(rig, tally, (p.who, "ship/order"), line);
    }
}

/// The block letter `tv` stands in, if it is on a site's ground.
fn rig_block(tw: &TestWorld, tv: ObjId) -> Option<char> {
    let (_, pose) = mush_sim::pose_of(tw.world(), tv)?;
    let (x, y) = tacsim::surface::site_cell(pose.x_cm, pose.y_cm);
    tacsim::surface::address::block_letter(x.div_euclid(4), y.div_euclid(4))
}

/// How many of block `letter`'s receipt-derived deposits are open (`claims.rs`'s `opened`).
fn opened_deposits(tw: &mut TestWorld, entry: &str, letter: char) -> usize {
    let families = tw
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry handles")
        .0
        .clone();
    tacsim::surface::geology::OreFamily::ALL
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
        .count()
}

/// A site's blocks, the most opened deposits first (then by letter).
fn blocks_by_deposits(tw: &mut TestWorld, entry: &str) -> Vec<char> {
    let mut ranked: Vec<(usize, char)> = tacsim::port::def::BLOCK_LETTERS
        .iter()
        .map(|letter| (opened_deposits(tw, entry, *letter), *letter))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|(_, letter)| letter).collect()
}

/// The Chebyshev distance between two blocks, in blocks.
fn block_distance(a: char, b: char) -> i64 {
    let (Some((ax, ay)), Some((bx, by))) = (
        tacsim::surface::address::block_of_letter(a),
        tacsim::surface::address::block_of_letter(b),
    ) else {
        return 0;
    };
    (ax - bx).abs().max((ay - by).abs())
}
