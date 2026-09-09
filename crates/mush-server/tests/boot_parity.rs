//! WS04-S2: the split half boots the roster the all-in-one boots.
//!
//! `mush-server::build_engine` composed the built-ins, worldgen, the NPC agent
//! and the selected character ruleset — and then stopped. `mushd` pushed one
//! more module after that list, the TacSim layer, and passed the operator's
//! `[tacsim.recovery]` policy into the built-ins where the split half passed
//! `RecoveryConfig::default()` and said so in a comment. So the forked half —
//! the PRODUCTION target — ran a world with `@tacsim`/`@unit` absent, a dead
//! `[tacsim.recovery]` block, and `TacsimRouteCatalog::resolve` answering `None`
//! for every route an operator's pack declared, which is `@journey/depart`
//! silently falling back to the fabricated manual route.
//!
//! These cases pin the composition through the one root both halves now call:
//!
//! - the order, as a golden list, with `tacsim` LAST — the claim
//!   `modules.rs`'s `e7c1_wiring_tacsim_composes_after_the_character_ruleset`
//!   makes about a hand-built list, now made about the roster a server boots;
//! - the layer really composes: a booted split engine loads the packs the
//!   config named, and a route resolves once its seed is applied;
//! - a pack that will not verify fails the split boot naming the file;
//! - the J5 archive pass is joined at shutdown rather than dropped detached;
//! - and (WS04-S3) `--check-config` finds each of those refusals WITHOUT
//!   booting, so an operator meets them before a supervisor does.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mush_config::config::Config;
use mush_core::{CanonicalId, Effects, GOD, GameModule, Inbound, Outbox};
use mush_llm::LlmService;
use mush_server::boot;
use mush_server::quarantine::QuarantineSpool;
use mush_server::sim_routes::TacsimRouteCatalog;
use mush_sim::RouteCatalog;
use mush_store::{InMemoryStore, ObjectStore};
use ruleset_tacsim::TacsimStore;

// ---- rig -------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mush-bootparity-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A memory-backed config on a scratch data dir, with the WoD ruleset selected —
/// `[rulesets].enabled`'s own default, and therefore the roster a stock server
/// composes.
fn wod_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.rulesets.enabled = vec!["wod".to_string()];
    cfg
}

/// Write the compiled-in corridor fixture out as the two pack FILES a host
/// configures, and point `[tacsim]` at them — the path an operator's own pack
/// takes, digests and all.
fn enable_tacsim(cfg: &mut Config) {
    let dir = cfg.server.data_dir.join("packs");
    std::fs::create_dir_all(&dir).unwrap();
    let core = dir.join("tacsim_core.json");
    let corridor = dir.join("tacsim_corridor.json");
    std::fs::write(&core, ruleset_tacsim::fixture::CORE_JSON).unwrap();
    std::fs::write(&corridor, ruleset_tacsim::fixture::CORRIDOR_JSON).unwrap();
    cfg.tacsim.enabled = true;
    cfg.tacsim.pack_paths = vec![core.display().to_string()];
    cfg.tacsim.seed_paths = vec![corridor.display().to_string()];
    cfg.tacsim.corridor = ruleset_tacsim::fixture::CORRIDOR_PACK.to_string();
}

fn llm_handle(cfg: &Config) -> Arc<LlmService> {
    let llm_config = mush_server::config_ext::llm_config(&cfg.llm).expect("[llm] parses");
    Arc::new(LlmService::from_config(&llm_config).expect("a keyless service still builds"))
}

fn spool(cfg: &Config) -> QuarantineSpool {
    QuarantineSpool::open(cfg.server.data_dir.join("quarantine")).expect("a fresh spool opens")
}

fn names(mods: &[Box<dyn GameModule>]) -> Vec<&str> {
    mods.iter().map(|m| m.name()).collect()
}

/// Type one line into a booted engine as God, and hand back what was said.
fn run(engine: &mut mush_core::Engine, line: &str) -> String {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 0,
            actor: GOD,
            line: line.to_string(),
        },
        &mut out,
        &mut fx,
    );
    out.take()
        .iter()
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- 1: the order ----------------------------------------------------------

/// The composition order, as a golden list.
///
/// `modules.rs` already asserts "TacSim composes after the character ruleset"
/// against a list its own test hand-builds. That assertion was true and the
/// shipped split root did not satisfy it, because the root never pushed the
/// module at all. This is the same claim made about the roster a server boots.
#[test]
fn game_modules_pins_the_composition_order() {
    let cfg = wod_config("order");
    let llm = llm_handle(&cfg);
    let mods = boot::game_modules(&cfg, &llm).expect("the stock roster composes");
    assert_eq!(
        names(&mods),
        [
            "comsys",
            "canon",
            "privileges",
            "org",
            "faction",
            "scenes",
            "sim",
            // S1: the atlas, right after the placement graph it will read (S2).
            "atlas",
            "tactics",
            "propagation",
            "economy",
            "inventory",
            "industry",
            "campaign",
            "mail",
            "jobs",
            "housing",
            "weather",
            "party",
            "market",
            "script",
            // The composition root's own three, after the built-ins.
            "worldgen",
            "npc",
            "wod",
            "tacsim",
            // S1: the content module after everything whose appliers it drives.
            "content",
        ],
        "the roster, and the one place its order is stated"
    );
    assert_eq!(
        names(&mods).iter().rev().nth(1).copied(),
        Some("tacsim"),
        "the TacSim layer must compose after the character ruleset: its boot reads that \
         ruleset's facts; only the content module, whose boot pass drives every other \
         module's appliers, comes after it"
    );
    assert_eq!(
        names(&mods).last().copied(),
        Some("content"),
        "the content module composes LAST: its boot pass writes other modules' RAM slots"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// `[tacsim.recovery]` reaches `mush-campaign` — which composes INSIDE the
/// built-ins — only through this root's mapping. The split half passed
/// `RecoveryConfig::default()` there and said so in a comment, so an operator
/// who enabled recovery on the forked half got a provider that answered "not
/// enabled" for the life of the deployment. `@recovery/status` is where that
/// shows, so that is what is asserted.
#[tokio::test]
async fn the_split_boot_carries_the_operator_recovery_policy() {
    let mut cfg = wod_config("recovery");
    enable_tacsim(&mut cfg);
    cfg.tacsim.recovery.enabled = true;
    cfg.tacsim.recovery.pool_account = 4242;
    cfg.tacsim.recovery.tiers = vec!["salvage".to_string()];
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = spool(&cfg);
    let mut booted = boot::build_engine(&cfg, &store, &spool)
        .await
        .expect("an enabled [tacsim.recovery] boots");

    let said = run(&mut booted.engine, "@recovery/status");
    assert!(
        !said.contains(mush_campaign::recoverycmd::NOT_ENABLED),
        "an enabled [tacsim.recovery] must reach the provider, not RecoveryConfig::default(): \
         {said}"
    );
    drop(booted);
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

// ---- S1: the starter area ---------------------------------------------------

/// The lot applies at a fresh world's first boot, lands in the store in the SAME
/// batch as its latch, and a second boot over that store loads it rather than
/// applying it again; a server with `[game].starter_area = false` boots Limbo and
/// God alone. No provider is configured on any of these boots, and none is needed.
#[tokio::test]
async fn the_starter_area_boots_once_and_is_durable() {
    let cfg = wod_config("starter");
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let quarantine = spool(&cfg);
    let rooms = |engine: &mush_core::Engine| {
        engine
            .world
            .iter()
            .filter(|o| o.kind == mush_core::ObjKind::Room)
            .count()
    };
    let booted = boot::build_engine(&cfg, &store, &quarantine)
        .await
        .expect("a fresh world boots with the starter area");
    assert_eq!(
        rooms(&booted.engine),
        mush_atlas::starter::LOT_ROOMS + 2,
        "fourteen lot rooms, the bus interior, and Limbo"
    );
    let objects = booted.engine.world.iter().count();
    for ns in ["atlas", "content"] {
        assert!(
            booted.archive_namespaces.iter().any(|n| n == ns),
            "{ns} is a composed module: {:?}",
            booted.archive_namespaces
        );
    }
    drop(booted);

    // The same store, booted again: the objects came back from the store and the
    // latch kept the packs from applying twice (P8).
    let again = boot::build_engine(&cfg, &store, &quarantine)
        .await
        .expect("a second boot over the same store");
    assert_eq!(rooms(&again.engine), mush_atlas::starter::LOT_ROOMS + 2);
    assert_eq!(
        again.engine.world.iter().count(),
        objects,
        "nothing applied twice"
    );
    let content = again
        .engine
        .world
        .module_state::<mush_content::ContentState>()
        .expect("the content module composed");
    assert_eq!(content.latches.all().count(), 3, "three latches came back");
    assert!(content.latches.all().all(|l| l.anomaly().is_none()));
    drop(again);

    let mut off = wod_config("starter-off");
    off.game.starter_area = false;
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool_off = spool(&off);
    let bare = boot::build_engine(&off, &store, &spool_off)
        .await
        .expect("boots without the starter area");
    assert_eq!(bare.engine.world.iter().count(), 2, "Limbo and God");
    drop(bare);
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
    let _ = std::fs::remove_dir_all(&off.server.data_dir);
}

// ---- 2: the layer really composes -----------------------------------------

/// A booted split engine holds the TacSim layer, enabled, with the operator's
/// packs loaded.
///
/// Deliberately NOT asserted: that the state slot exists. `TacsimRuleset`
/// installs it unconditionally ("the state slot always exists, even disabled"),
/// so that assertion passes against the very tree this step fixes.
#[tokio::test]
async fn split_build_engine_composes_the_tacsim_layer() {
    let mut cfg = wod_config("layer");
    enable_tacsim(&mut cfg);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = spool(&cfg);

    let booted = boot::build_engine(&cfg, &store, &spool)
        .await
        .expect("the split half boots with [tacsim].enabled");

    let tacsim = booted
        .engine
        .world
        .module_state::<TacsimStore>()
        .expect("the layer composed");
    assert!(
        tacsim.enabled(),
        "the split boot must carry the operator's [tacsim].enabled through to the module"
    );
    assert!(
        tacsim.packs().next().is_some(),
        "the packs [tacsim].pack_paths named were not loaded on the split half"
    );
    assert!(
        booted.archive_namespaces.iter().any(|ns| ns == "tacsim"),
        "the archive namespace list must cover every composed module: {:?}",
        booted.archive_namespaces
    );
    drop(booted);
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// …and once the seed is applied, a route the pack declares RESOLVES.
///
/// This is the observable the split half lost. `TacsimRouteCatalog::resolve`
/// answers `None` until an applied seed latch has placed every place a route
/// names — and with no TacSim module composed at all it could never answer
/// anything else, so `@journey/depart <route>` fell back to the fabricated
/// manual route with zero declared propellant. A bare boot is therefore NOT
/// evidence: the seed has to be applied first, which is what this drives.
#[tokio::test]
async fn a_seeded_pack_resolves_a_route() {
    let mut cfg = wod_config("route");
    enable_tacsim(&mut cfg);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = spool(&cfg);
    let mut booted = boot::build_engine(&cfg, &store, &spool)
        .await
        .expect("the split half boots with [tacsim].enabled");

    let route = CanonicalId::new(ruleset_tacsim::fixture::ROUTE).expect("a well-formed id");
    let catalog = TacsimRouteCatalog;
    assert!(
        catalog.resolve(&booted.engine.world, &route).is_none(),
        "before the seed is applied nothing is placed, so nothing resolves"
    );

    let said = run(
        &mut booted.engine,
        &format!(
            "@tacsim/seed {} commit",
            ruleset_tacsim::fixture::CORRIDOR_PACK
        ),
    );

    let resolved = catalog.resolve(&booted.engine.world, &route);
    assert!(
        resolved.is_some(),
        "a seeded pack's route must resolve through the composed layer; \
         the seed answered: {said}"
    );
    drop(booted);
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

// ---- 3: a bad pack fails the split boot ------------------------------------

/// A pack whose bytes no longer match its declared digest refuses the SPLIT
/// boot, naming the file.
///
/// Only the observable contract is asserted. Two sites can refuse — the host
/// reads the packs in `config_ext::tacsim_config`, and `TacsimModule::register`
/// independently returns its cached load error — and which one fires depends on
/// construction, so the test pins the operator-facing half: the boot stops, and
/// the message says which file.
#[tokio::test]
async fn a_digest_mismatched_pack_fails_the_split_boot() {
    let mut cfg = wod_config("tampered");
    enable_tacsim(&mut cfg);
    let path = PathBuf::from(&cfg.tacsim.pack_paths[0]);
    let tampered = std::fs::read_to_string(&path)
        .unwrap()
        .replace("alloy_smelt", "alloy_smelts");
    std::fs::write(&path, tampered).unwrap();

    let error = mush_server::serve_with_store(cfg.clone(), None, None)
        .await
        .expect_err("a pack that will not verify must refuse the split boot");
    assert!(
        error.contains("tacsim_core.json"),
        "the refusal must name the offending file: {error}"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

// ---- 5: the archive pass is joined, not detached ---------------------------

/// The shutdown tail JOINS the J5 pass.
///
/// The pass never ends on its own — it sleeps in a loop for the life of the
/// process — so "joined" has to mean cancel-then-await, and the failure mode is
/// a caller that drops the handle and returns with the task still live inside a
/// runtime it is about to tear down. A dropped `JoinHandle` does not stop a
/// tokio task, so the observable is simply whether the task is still doing work
/// afterwards: this stands in a ticking task for the pass and requires it to
/// have STOPPED when `stop_ledger_archive` returns.
#[tokio::test]
async fn stop_ledger_archive_joins_the_pass_rather_than_detaching_it() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ticks);
    // The shape of the real pass: a sleep loop with no shutdown seam of its own.
    let job = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(1)).await;
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        ticks.load(Ordering::SeqCst) > 0,
        "the stand-in pass never ran, so this case would prove nothing"
    );

    boot::stop_ledger_archive(Some(job)).await;
    let after_stop = ticks.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        ticks.load(Ordering::SeqCst),
        after_stop,
        "stop_ledger_archive returned with the pass still running: the handle was detached"
    );

    // `None` (the tier is off) is a no-op, not a panic.
    boot::stop_ledger_archive(None).await;
}

/// …and the tail's join does not wedge a real shutdown.
///
/// A tail that awaited the pass WITHOUT cancelling it would hang here forever,
/// because the pass has no shutdown seam: this boots the split server with the
/// archive tier armed (the `open_store` branch — no caller-supplied store), lets
/// a gateway connect and disconnect, and requires `serve` to return.
#[tokio::test]
async fn the_split_shutdown_awaits_the_archive_job() {
    let mut cfg = wod_config("shutdown");
    cfg.store.archive.enabled = true;
    let data_dir = cfg.server.data_dir.clone();
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();

    // `serve`'s future is not `Send` (the module list is not `Sync`), so the
    // server gets its own thread and its own runtime — the same rig the other
    // split-process tests boot through.
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = done_tx.send(runtime.block_on(mush_server::serve(cfg, Some(bound_tx))));
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(60), bound_rx)
        .await
        .expect("the split server bound its socket")
        .expect("the bound endpoint");

    let (reader, writer) = mush_ipc::connect(&endpoint).await.unwrap();
    let mut frames = mush_ipc::FrameReader::new(reader);
    assert!(matches!(
        frames.read::<mush_ipc::ToGateway>().await.unwrap().unwrap(),
        mush_ipc::ToGateway::Hello { .. }
    ));
    // Clean EOF: the boundary is gone, so the server shuts the world down and
    // walks its whole tail — including the archive join.
    drop(frames);
    drop(writer);

    done_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("the shutdown tail must not wedge on the never-ending archive pass")
        .expect("a clean EOF is an ordinary exit");
    let _ = std::fs::remove_dir_all(&data_dir);
}

// ---- 6: --check-config finds the world half's refusals without booting ------

/// WS04-S3. `boot::check_config` is what `mushd --check-config` and
/// `mush-server --check-config` run, and its whole value is that it refuses the
/// same values `build_engine`/`spawn_ledger_archive` refuse — at a cost of
/// nothing, against a config a live instance may already be serving.
///
/// So it is asserted against the two refusals only the WORLD half can find, and
/// each is checked to actually name what it refused: a preflight that says "no"
/// without saying which line is no better than the boot failure it replaces.
#[test]
fn check_config_finds_the_world_half_refusals_without_booting() {
    // A stock config preflights clean, or the flag is a thing that always says
    // no and no operator would run it twice.
    let mut cfg = wod_config("check-clean");
    assert!(
        boot::check_config(&cfg).is_empty(),
        "{:?}",
        boot::check_config(&cfg)
    );

    // (a) a pack whose bytes no longer match its declared digest — the refusal
    // `a_digest_mismatched_pack_fails_the_split_boot` proves the boot makes,
    // made here without opening the store.
    let mut tampered_cfg = wod_config("check-tampered");
    enable_tacsim(&mut tampered_cfg);
    let path = PathBuf::from(&tampered_cfg.tacsim.pack_paths[0]);
    let tampered = std::fs::read_to_string(&path)
        .unwrap()
        .replace("alloy_smelt", "alloy_smelts");
    std::fs::write(&path, tampered).unwrap();
    let refusals = boot::check_config(&tampered_cfg);
    assert!(
        refusals.iter().any(|r| r.contains("tacsim_core.json")),
        "the preflight must name the offending pack: {refusals:?}"
    );

    // (b) an unparseable archive window. `spawn_ledger_archive` refuses the
    // boot on it rather than leaving the nightly pass silently unscheduled for
    // the life of the deployment — which makes it exactly the kind of value
    // worth finding ahead of a restart.
    cfg.store.archive.enabled = true;
    cfg.store.archive.window = "25:61-03:00".to_owned();
    let refusals = boot::check_config(&cfg);
    assert!(
        refusals.iter().any(|r| r.contains("25:61")),
        "the preflight must name the offending window: {refusals:?}"
    );
    // ...and a disabled tier does not care what the window says, because
    // nothing will ever read it.
    cfg.store.archive.enabled = false;
    assert!(boot::check_config(&cfg).is_empty());

    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
    let _ = std::fs::remove_dir_all(&tampered_cfg.server.data_dir);
}
