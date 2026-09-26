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
//!
//! WS04-S4 finishes the job in the direction the fork actually runs: the SPLIT
//! root is the reference and the all-in-one is measured against it, because the
//! split half is the production target and `mushd` is the local rig. Four
//! groups are added here:
//!
//! - the roster and `build_hello_manifest` goldens over the one production
//!   composition x `{tacsim off | on}`, so a module added, dropped or reordered
//!   fails a named test with `boot.rs` in the message rather than changing what
//!   a client is told in silence. (16b cut R-b collapsed the `{no ruleset | wod
//!   | shadowrun}` half of that matrix: the two character rulesets are gone and
//!   the one game sheet composes unconditionally, so there is exactly one
//!   composition left to state a golden for.);
//! - the config-read coverage table below — every boot-relevant knob set to a
//!   non-default value with an observable asserted, on EVERY entry point that
//!   reads it, including the `serve_with_store` caller-supplied-store branch
//!   where `[store.archive]` is deliberately ignored and says so;
//! - the one-root tripwire: `Engine::compose_modules(` and `panel_feeds()`
//!   appear exactly ONCE across `mush-server/src/{lib,boot}.rs` and
//!   `mushd/src/{main,boot}.rs`, so a second transcription of the composition
//!   root cannot land quietly (`mushd/tests/boot_composition.rs`'s own
//!   tripwire makes the narrower "mushd re-states nothing" claim; this one
//!   counts the whole fork);
//! - and the three `mushd` rigs that used to rebuild the manifest by hand now
//!   call [`boot::build_hello_manifest`], which is what makes it the single
//!   producer it is documented as being.
//!
//! # The config-read coverage table
//!
//! | knob | entry point(s) | the case that proves it |
//! |------|----------------|-------------------------|
//! | `[store].receipt_policy` | `open_store` (both roots) | `the_store_knobs_are_read_where_the_store_is_opened` |
//! | `[store.archive].enabled` | `open_store` | `the_store_knobs_are_read_where_the_store_is_opened` |
//! | `[store.archive].enabled` | `serve_with_store` + caller store | `a_caller_supplied_store_says_the_archive_tier_is_ignored` |
//! | `[store.archive].enabled` | `serve` (shutdown tail) | `the_split_shutdown_awaits_the_archive_job` |
//! | `[store.archive].window` | `spawn_ledger_archive` / `check_config` | `check_config_finds_the_world_half_refusals_without_booting` |
//! | `[atlas].pack_paths` | `game_modules` -> `build_engine` / `check_config` | `check_config_finds_the_world_half_refusals_without_booting` |
//! | `[tacsim].enabled` / `.pack_paths` | `game_modules` -> `build_engine` | `split_build_engine_composes_the_tacsim_layer`, the matrix |
//! | `[tacsim].seed_paths` | `build_engine` + an applied seed | `a_seeded_pack_resolves_a_route` |
//! | `[tacsim].recovery` | `game_modules` (into the built-ins) | `the_split_boot_carries_the_operator_recovery_policy` |
//! | `[tacsim].ground_pulse_ms` / `.space_pulse_ms` | `build_engine` post-composition | `the_engine_knobs_reach_the_booted_world` |
//! | `[llm].decision_workers` | `build_engine` -> `BootedEngine` | `the_engine_knobs_reach_the_booted_world` |
//! | `[game].max_characters` | `build_engine` -> `@newchar` | `the_engine_knobs_reach_the_booted_world` |
//! | `[offers].key_dir`, `[server].data_dir` (decision keys) | `build_engine` key rings | `the_engine_knobs_reach_the_booted_world` |
//! | `[tacsim.economy].housing` / `.market` / `.reserved_rooms` | `game_modules` -> `server_modules_with` | `modules.rs`'s `lb_e5cc231_…`, `tests/market_issuer.rs` |
//!
//! A knob added to `mush-config` and never wired into `boot.rs` leaves a row
//! missing from this table — a documented, greppable gap rather than a silent
//! one — and a knob wired into one entry point only (the shape this workstream
//! exists to abolish) is a missing ROW, not a missing test file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_config::config::Config;
use mush_core::{CanonicalId, Effects, GOD, GameModule, Inbound, Outbox};
use mush_llm::LlmService;
use mush_server::boot::{self, BootedEngine};
use mush_server::quarantine::QuarantineSpool;
use mush_server::sim_routes::TacsimRouteCatalog;
use mush_server::staff::StaffOpKind;
use mush_sim::RouteCatalog;
use mush_store::{InMemoryStore, ObjectStore};
use tacsim::TacsimStore;

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

/// A memory-backed config on a scratch data dir — the roster a stock server
/// composes.
///
/// 16b cut R-b: there is nothing to select. `[rulesets].enabled` selected one of
/// two character rulesets and is read by nothing now, so the stock config IS the
/// production composition.
fn stock_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ipc.listen = "127.0.0.1:0".to_string();
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
    std::fs::write(&core, tacsim::fixture::CORE_JSON).unwrap();
    std::fs::write(&corridor, tacsim::fixture::CORRIDOR_JSON).unwrap();
    cfg.tacsim.enabled = true;
    cfg.tacsim.pack_paths = vec![core.display().to_string()];
    cfg.tacsim.seed_paths = vec![corridor.display().to_string()];
    cfg.tacsim.corridor = tacsim::fixture::CORRIDOR_PACK.to_string();
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

/// One booted split engine and the config it was booted from, on a scratch data
/// dir this cleans up on drop.
///
/// Every case below is a full [`boot::build_engine`] — three key rings, the K3
/// recovery barrier, the writable bootstrap, migrations, the world load and
/// every module's boot pass — so the rig is shared or the suite is
/// unmaintainable. Roughly a dozen boots run here; on the memory backend each
/// costs a few tens of milliseconds, which is the accepted price of asserting
/// the composition a server actually reaches rather than one a test hand-builds.
struct Fixture {
    cfg: Config,
    booted: BootedEngine,
    /// Held for the fixture's life: the store the engine was booted against,
    /// and the K0 spool whose lock covers the scratch data dir.
    _store: Arc<dyn ObjectStore>,
    _spool: QuarantineSpool,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cfg.server.data_dir);
    }
}

/// Boot the SPLIT root — the production half, and therefore the reference this
/// whole file measures against — over a stock memory-backed config `mutate` may
/// adjust first.
async fn boot_fixture(tag: &str, mutate: impl FnOnce(&mut Config)) -> Fixture {
    let mut cfg = stock_config(tag);
    mutate(&mut cfg);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = spool(&cfg);
    let booted = boot::build_engine(&cfg, &store, &spool)
        .await
        .expect("the split half boots");
    Fixture {
        cfg,
        booted,
        _store: store,
        _spool: spool,
    }
}

/// The roster the composition root builds — **the golden**, and the one place
/// this suite states it.
///
/// The built-ins first (`modules::server_modules_with`), then the root's own
/// worldgen and NPC modules, then the one game sheet and its rules interface,
/// then the TacSim layer, then content. `tacsim` is in the list whether or not
/// `[tacsim].enabled`: the layer always composes (its state slot always
/// exists), and the flag decides what it DOES, not whether a client is told
/// about it.
///
/// 16b cut R-b: the `Option<&str>` selection is gone with the two character
/// rulesets. `pilot` and `rules_interface` are now unconditional rows.
fn expected_roster() -> Vec<&'static str> {
    let mut roster = vec![
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
        // The composition root's own two, after the built-ins.
        "worldgen",
        "npc",
    ];
    // ...then the one game sheet, and (S6) the rules interface the server
    // composes beside it — both unconditional since 16b cut R-b.
    roster.push("pilot");
    roster.push("rules_interface");
    // ...the TacSim layer next: its boot reads the character sheet's facts.
    roster.push("tacsim");
    // ...and the content module LAST: its boot pass drives every other module's
    // appliers, so every owner must have hydrated before it runs (S1).
    roster.push("content");
    roster
}

/// The panel feeds the composition advertises, in registration order, plus the
/// two `build_hello_manifest` appends engine-side.
fn expected_panels() -> Vec<&'static str> {
    let mut panels = vec![
        "commands.list",
        "comsys",
        "scenes",
        // S1/S4: the atlas composes after `mush-sim` (which advertises no feed)
        // and before `mush-tactics`, and the map frame is the feed it publishes.
        "atlas.map",
        "tactics.unit",
        "tactics.vitals",
        "tactics.contacts",
        "mail",
        "weather",
        "worldgen.draft",
        "llm.dashboard",
        "npc.thinking",
        "intel",
    ];
    // 16b cut R-b: the one game sheet's ONE feed, where the selected ruleset's
    // block of three used to sit. It is named for the sheet SCHEMA's ruleset id
    // (`tacsim`) and not for the module (`pilot`): `register_sheet_commands`
    // reserves `<schema.ruleset>.sheet`. There is deliberately no `.roll` feed
    // and no `.vitals` feed — doc 16 §3.3 leaves M32's `TacticalUnit` cells the
    // one damage authority, so the sheet declares no health track to publish.
    panels.push("tacsim.sheet");
    // 16b cut T.2: the arena's cockpit frame feed, registered by the TacSim LAYER
    // (module `tacsim`) rather than by the sheet (module `pilot`) — hence after
    // `tacsim.sheet` in registration order. It is registered whether or not
    // `[tacsim].enabled` is set, for the same reason the layer's verbs are: a
    // disabled layer refuses out loud rather than making a feed appear and
    // disappear under the web client's manifest. Cut K's cockpit is the consumer.
    panels.push("arena.cockpit");
    // 16b cut Q.3: the terrain feed, registered immediately after the cockpit
    // frame's in `arena_bridge::register` and therefore listed here next. It is
    // pull-only — nothing is ever pushed on it — but it is a registered feed so
    // that it reaches the client manifest and so that a `PanelSubscribe` routes
    // to its query handler.
    panels.push("arena.map");
    panels.push("who");
    panels.push("legal.actions");
    panels
}

/// A `MakeWriter` that collects formatted log lines into a shared buffer.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the capture buffer")).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("the capture buffer")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
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
    let cfg = stock_config("order");
    let llm = llm_handle(&cfg);
    let mods = boot::game_modules(&cfg, &llm).expect("the stock roster composes");
    assert_eq!(
        names(&mods),
        expected_roster(),
        "the roster, and the one place its order is stated"
    );
    assert_eq!(
        names(&mods).iter().rev().nth(1).copied(),
        Some("tacsim"),
        "the TacSim layer must compose after the character sheet: its boot reads that \
         sheet's facts; only the content module, whose boot pass drives every other \
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
    let mut fx = boot_fixture("recovery", |cfg| {
        enable_tacsim(cfg);
        cfg.tacsim.recovery.enabled = true;
        cfg.tacsim.recovery.pool_account = 4242;
        cfg.tacsim.recovery.tiers = vec!["salvage".to_string()];
    })
    .await;

    let said = run(&mut fx.booted.engine, "@recovery/status");
    assert!(
        !said.contains(mush_campaign::recoverycmd::NOT_ENABLED),
        "an enabled [tacsim.recovery] must reach the provider, not RecoveryConfig::default(): \
         {said}"
    );
}

// ---- 1b: the roster and the hello manifest, across the matrix --------------

/// The roster AND the web `hello` manifest, as goldens, over the one production
/// composition x `{tacsim off | on}` — measured on the SPLIT root, which is the
/// production half and therefore the reference the all-in-one is equal to,
/// rather than the other way round.
///
/// 16b cut R-b collapsed the `{no ruleset | wod | shadowrun}` half of this
/// matrix. It used to make the claim *"`[rulesets].enabled` moves exactly one
/// module and one block of panel feeds, and moves nothing else"*; there is one
/// composition now, and what is left of that claim is stated positively — the
/// sheet is `pilot` and `rules_interface`, and its panel feeds are exactly
/// `tacsim.sheet` (see [`expected_roster`] and [`expected_panels`]).
///
/// The claim only a matrix can make survives whole:
///
/// - `[tacsim].enabled` moves NOTHING in the manifest. The layer composes
///   either way (its state slot always exists), so the feature is advertised
///   either way and the flag decides what the layer DOES — which is why
///   enabled-ness is asserted on the module state beside the manifest rather
///   than inferred from it. Before WS04-S2 the split root pushed no TacSim
///   module at all and its `Hello` could not carry the feature; that it now
///   does is asserted here in its own right.
///
/// The failure message names `crates/mush-server/src/boot.rs` because that is
/// the one file to edit when a module is added on purpose.
#[tokio::test]
async fn the_roster_and_hello_manifest_are_golden_across_the_matrix() {
    for tacsim_on in [false, true] {
        let tag = format!("matrix-{tacsim_on}");
        let fx = boot_fixture(&tag, |cfg| {
            if tacsim_on {
                enable_tacsim(cfg);
            }
        })
        .await;
        let case = format!("[tacsim].enabled = {tacsim_on}");

        let roster = names(fx.booted.engine.modules());
        assert_eq!(
            roster,
            expected_roster(),
            "the booted roster drifted from the golden ({case}); if that was \
             deliberate, `game_modules` in crates/mush-server/src/boot.rs is the \
             one file to edit"
        );

        let (features, panels) = boot::build_hello_manifest(&fx.booted.engine);
        let features: Vec<&str> = features.iter().map(String::as_str).collect();
        let expected_features: Vec<&str> = expected_roster()
            .into_iter()
            .map(|name| if name == "comsys" { "channels" } else { name })
            .collect();
        assert_eq!(
            features, expected_features,
            "the web hello FEATURES are the roster with comsys advertised as \
             channels, and nothing else ({case})"
        );
        let panels: Vec<&str> = panels.iter().map(String::as_str).collect();
        assert_eq!(
            panels,
            expected_panels(),
            "the web hello PANELS drifted from the golden ({case}): a module \
             registered or dropped a panel feed, and every web client was told so \
             in silence"
        );
        assert!(
            features.contains(&"tacsim"),
            "the SPLIT root's hello must advertise the tacsim layer ({case}): \
             before WS04-S2 it composed no such module and could not"
        );
        // …and `[tacsim]` still moves NO module: the game sheet composes on both
        // arms, which is the half of the old matrix's claim that survives.
        assert!(
            features.contains(&"pilot") && features.contains(&"rules_interface"),
            "the one game sheet composes whatever [tacsim] says ({case}): {features:?}"
        );

        let tacsim = fx
            .booted
            .engine
            .world
            .module_state::<TacsimStore>()
            .expect("the layer composes whatever [tacsim].enabled says");
        assert_eq!(
            tacsim.enabled(),
            tacsim_on,
            "[tacsim].enabled decides what the composed layer DOES, and that is \
             the only thing it decides ({case})"
        );
    }
}

// ---- S1: the starter area ---------------------------------------------------

/// The lot applies at a fresh world's first boot, lands in the store in the SAME
/// batch as its latch, and a second boot over that store loads it rather than
/// applying it again; a server with `[game].starter_area = false` boots Limbo and
/// God alone. No provider is configured on any of these boots, and none is needed.
#[tokio::test]
async fn the_starter_area_boots_once_and_is_durable() {
    let cfg = stock_config("starter");
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

    let mut off = stock_config("starter-off");
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
    let fx = boot_fixture("layer", enable_tacsim).await;

    let tacsim = fx
        .booted
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
        fx.booted.archive_namespaces.iter().any(|ns| ns == "tacsim"),
        "the archive namespace list must cover every composed module: {:?}",
        fx.booted.archive_namespaces
    );
}

/// [`boot_fixture`] with the boot's own log captured at `INFO`.
///
/// The same device as `a_caller_supplied_store_says_the_archive_tier_is_ignored`
/// below, at a lower level: the only observable of "the boot said so" is the log,
/// so the log is what these read.
async fn boot_saying(tag: &str, mutate: impl FnOnce(&mut Config)) -> (String, Fixture) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(capture.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let fx = boot_fixture(tag, mutate).await;
    drop(guard);
    (capture.text(), fx)
}

/// **The boot says which way the layer went, and whether the centre is ready**
/// (16b cut F.4).
///
/// Near-miss headers are refused by Config::load. A missing or disabled section
/// warns, and an enabled centre reports both readiness and any configured slot
/// ceiling clamped to its available pods.
#[tokio::test]
async fn the_boot_says_whether_the_simulator_is_there() {
    let (said, fx) = boot_saying("tacsim-log-on", enable_tacsim).await;
    assert!(
        said.contains("tacsim enabled: the simulator's arena and its stock tanks apply at boot"),
        "the layer went on and the boot must say so: {said}"
    );
    assert!(
        said.contains("simulator centre ready: the arena is applied and its tanks are standing"),
        "and then whether the centre is whole: {said}"
    );
    assert!(
        said.contains("roster_tanks=12 of 12 pods=12 of 12"),
        "doc 16 §3.3's stock tanks and the centre's pods, both counted against the twelve \
         16b cut Z.3 seats: {said}"
    );
    assert!(
        !said.contains("NOT ready") && !said.contains("partly ready"),
        "a whole centre must not warn: {said}"
    );
    assert_eq!(
        said.matches("simulator slots clamped").count(),
        1,
        "the default ceiling exceeds the six pods and must warn once: {said}"
    );
    assert!(said.contains("configured_slots_per_side=12"), "{said}");
    assert!(said.contains("available_pods_per_side=6"), "{said}");
    drop(fx);

    let (said, fx) = boot_saying("tacsim-log-no-clamp", |cfg| {
        enable_tacsim(cfg);
        cfg.tacsim.match_.max_slots_per_side = 6;
    })
    .await;
    assert!(!said.contains("simulator slots clamped"), "{said}");
    drop(fx);

    // A deliberately disabled layer remains supported and warns.
    let (said, fx) = boot_saying("tacsim-log-off", |cfg| cfg.tacsim.enabled = false).await;
    assert!(
        said.contains("tacsim disabled ([tacsim].enabled is false or the section is missing)"),
        "the negative is the line an operator greps for: {said}"
    );
    assert!(
        said.lines()
            .any(|line| line.contains("WARN") && line.contains("tacsim disabled")),
        "the disabled layer must warn: {said}"
    );
    assert!(
        !said.contains("simulator centre ready") && !said.contains("simulator centre NOT ready"),
        "with no layer there is no arena, so there is no readiness either way to report: {said}"
    );
    drop(fx);
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
    let mut fx = boot_fixture("route", enable_tacsim).await;

    let route = CanonicalId::new(tacsim::fixture::ROUTE).expect("a well-formed id");
    let catalog = TacsimRouteCatalog;
    assert!(
        catalog.resolve(&fx.booted.engine.world, &route).is_none(),
        "before the seed is applied nothing is placed, so nothing resolves"
    );

    let said = run(
        &mut fx.booted.engine,
        &format!("@tacsim/seed {} commit", tacsim::fixture::CORRIDOR_PACK),
    );

    let resolved = catalog.resolve(&fx.booted.engine.world, &route);
    assert!(
        resolved.is_some(),
        "a seeded pack's route must resolve through the composed layer; \
         the seed answered: {said}"
    );
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
    let mut cfg = stock_config("tampered");
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
    let mut cfg = stock_config("shutdown");
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
    let mut cfg = stock_config("check-clean");
    assert!(
        boot::check_config(&cfg).is_empty(),
        "{:?}",
        boot::check_config(&cfg)
    );

    // (a) a pack whose bytes no longer match its declared digest — the refusal
    // `a_digest_mismatched_pack_fails_the_split_boot` proves the boot makes,
    // made here without opening the store.
    let mut tampered_cfg = stock_config("check-tampered");
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

    // (c) S1: `[atlas].pack_paths` is host-read and boot-fatal exactly as
    // `[tacsim].pack_paths` is, so the preflight has to find it too — and name
    // the file, which is the whole reason the atlas reads its packs host-side.
    let mut atlas_cfg = stock_config("check-atlas");
    let missing = PathBuf::from(&atlas_cfg.server.data_dir).join("no-such-pack.json");
    atlas_cfg.atlas.pack_paths = vec![missing.display().to_string()];
    let refusals = boot::check_config(&atlas_cfg);
    assert!(
        refusals.iter().any(|r| r.contains("no-such-pack.json")),
        "the preflight must name the offending atlas pack: {refusals:?}"
    );
    let _ = std::fs::remove_dir_all(&atlas_cfg.server.data_dir);

    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
    let _ = std::fs::remove_dir_all(&tampered_cfg.server.data_dir);
}

// ---- 7: the config-read coverage table -------------------------------------

/// Every boot-relevant knob the ENGINE half owns, set to a non-default value in
/// one boot, each with an observable asserted.
///
/// The table in this file's header is the index; this is the row-set that needs
/// a booted world. A knob that parses, validates and then reaches nothing is the
/// failure this catches: `[llm].decision_workers` fed a worker pool that was
/// built from a second reading of the stanza, `[tacsim].ground_pulse_ms` was
/// "parsed and inert" for two cuts, and `[game].max_characters` is enforced by a
/// closure captured at registration — none of which any config test can see.
#[tokio::test]
async fn the_engine_knobs_reach_the_booted_world() {
    let mut fx = boot_fixture("knobs", |cfg| {
        cfg.llm = toml::from_str("decision_workers = 5").expect("[llm] is a table");
        cfg.game.max_characters = 7;
        cfg.tacsim.ground_pulse_ms = 5_000;
        cfg.tacsim.space_pulse_ms = 11_000;
        cfg.offers.key_dir = Some(cfg.server.data_dir.join("secrets").join("offers"));
    })
    .await;

    // `[llm].decision_workers` — read once at boot and carried on the booted
    // engine, so the serving half never re-parses `[llm]` to find it.
    assert_eq!(
        fx.booted.decision_workers, 5,
        "[llm].decision_workers must reach the serving half through BootedEngine"
    );

    // `[tacsim].ground_pulse_ms` / `.space_pulse_ms` — the post-composition
    // upgrade of the sim's live cadence (the same claim mushd's own boot suite
    // makes about the all-in-one; this is the split half's).
    let pulse = fx
        .booted
        .engine
        .world
        .module_state::<mush_sim::tick::PulseConfig>()
        .copied()
        .expect("the sim composes the pulse slot");
    assert_eq!(
        (pulse.ground_ms, pulse.space_ms),
        (5_000, 11_000),
        "the split root must upgrade the pulse cadence from [tacsim]"
    );

    // `[offers].key_dir` — the K5 ring goes where the operator put it, and the
    // K7 decision ring (which has no override) goes under `[server].data_dir`.
    let offers_dir = fx.cfg.offer_key_dir();
    assert!(
        std::fs::read_dir(&offers_dir).is_ok_and(|d| d.count() > 0),
        "[offers].key_dir must hold the provisioned ring: {}",
        offers_dir.display()
    );
    assert!(
        std::fs::read_dir(fx.cfg.decision_key_dir()).is_ok_and(|d| d.count() > 0),
        "the K7 decision ring lives under [server].data_dir"
    );

    // `[game].max_characters` — captured by the `@newchar` handler at
    // registration, so the only place it is visible is the op the handler
    // queues for the off-loop executor.
    let said = run(&mut fx.booted.engine, "@newchar Rosalind");
    let queued = fx.booted.staff.drain();
    let cap = queued.iter().find_map(|op| match &op.kind {
        StaffOpKind::NewChar { max_characters, .. } => Some(*max_characters),
        _ => None,
    });
    assert_eq!(
        cap,
        Some(7),
        "[game].max_characters must reach the @newchar handler; it said: {said}"
    );
}

/// The `[store]` knobs, on the entry point that reads them — [`boot::open_store`],
/// which both roots call and which the caller-supplied-store branch skips.
///
/// `receipt_policy` is asserted through its refusals because that is the whole
/// point of the knob: an unparseable value must name itself rather than
/// downgrade to `applied` in silence, and `host_power_loss` on the memory
/// backend is a promise nothing in RAM can keep.
#[tokio::test]
async fn the_store_knobs_are_read_where_the_store_is_opened() {
    let mut cfg = stock_config("store-knobs");

    // A spelling the parser does not know refuses, naming the value.
    cfg.store.receipt_policy = "hostpowerloss".to_owned();
    let error = boot::open_store(&cfg)
        .await
        .expect_err("an unknown receipt policy must refuse the boot");
    assert!(
        error.contains("hostpowerloss"),
        "the refusal must name the offending value: {error}"
    );

    // ...and the strongest policy is refused outright on the memory backend.
    cfg.store.receipt_policy = "host_power_loss".to_owned();
    let error = boot::open_store(&cfg)
        .await
        .expect_err("host_power_loss on memory is unkeepable");
    assert!(
        error.contains("requires the Redis backend"),
        "the refusal must say the policy needs Redis: {error}"
    );
    cfg.store.receipt_policy = "applied".to_owned();

    // `[store.archive].enabled` opens the tier under `[server].data_dir` and
    // hands the handle back for the nightly pass. The split root reached this
    // branch for the first time in WS04-S1; before that `[store.archive]` was
    // an all-in-one-only knob.
    assert!(
        boot::open_store(&cfg)
            .await
            .expect("the stock config opens")
            .archive
            .is_none(),
        "the tier is off by default, so nothing is opened"
    );
    cfg.store.archive.enabled = true;
    let opened = boot::open_store(&cfg)
        .await
        .expect("an enabled archive tier opens");
    assert!(
        opened.archive.is_some(),
        "[store.archive].enabled must attach a LedgerArchive on the SPLIT root"
    );
    assert!(
        cfg.server.data_dir.join(&cfg.store.archive.dir).is_dir(),
        "the tier lives under [server].data_dir/{}",
        cfg.store.archive.dir
    );

    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// The other entry point for the same knob: a caller who brought their OWN
/// store never reaches [`boot::open_store`], so `[store.archive]` composes
/// nothing and schedules nothing — and WS04-S2 made that branch SAY so.
///
/// This is the row the coverage table exists for. A knob that works through one
/// entry point and silently does nothing through another is the exact shape
/// this workstream abolishes, and the only observable of "said out loud" is the
/// log line, so the log is what this reads.
///
/// The warn is emitted on the thread that drives `serve_with_store`'s future to
/// the bind point, which is this test's own server thread under its own scoped
/// subscriber — and no other case in this binary reaches that callsite, so the
/// interest cache cannot be poisoned by a sibling running first.
#[tokio::test]
async fn a_caller_supplied_store_says_the_archive_tier_is_ignored() {
    let mut cfg = stock_config("archive-ignored");
    cfg.store.archive.enabled = true;
    let data_dir = cfg.server.data_dir.clone();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let capture = Capture::default();
    let sink = capture.clone();
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();

    // `serve`'s future is not `Send`, so the server gets its own thread and its
    // own runtime — the same rig the other split-process cases boot through.
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(sink)
            .finish();
        let served = tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(mush_server::serve_with_store(
                cfg,
                Some(bound_tx),
                Some(store),
            ))
        });
        let _ = done_tx.send(served);
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(60), bound_rx)
        .await
        .expect("the split server bound its socket")
        .expect("the bound endpoint");

    let logged = capture.text();
    assert!(
        logged.contains("[store.archive].enabled is ignored on this boot"),
        "a caller-supplied store disarms the archive tier, and the boot must say so \
         rather than leave an operator watching a ledger that never stops growing; \
         the boot logged: {logged}"
    );

    // Clean EOF ends the server, so the thread and its scratch dir go away.
    let (reader, writer) = mush_ipc::connect(&endpoint).await.unwrap();
    drop(reader);
    drop(writer);
    let _ = done_rx.recv_timeout(Duration::from_secs(60));
    let _ = std::fs::remove_dir_all(&data_dir);
}

// ---- 8: one composition root, across the whole fork ------------------------

/// `Engine::compose_modules(` and `panel_feeds()` each appear EXACTLY ONCE
/// across the four files that boot a world: `mush-server/src/{lib,boot}.rs` and
/// `mushd/src/{main,boot}.rs`.
///
/// Two transcriptions of one boot do not disagree loudly — they disagree by
/// omission, which is how the all-in-one came to run a world with no decision
/// key ring while the split half ran one with it, and how the split half came to
/// advertise a hello manifest the all-in-one assembled differently. `mushd`'s
/// own suite makes the narrower claim (`boot_composition.rs`: mushd re-states
/// nothing); this one counts the whole fork, so a SECOND composition or a second
/// manifest assembly cannot land on either side.
///
/// Scoped to exactly these four files and no wider on purpose:
/// `mush-contract-tests/tests/legacy_handler_allowlist.rs` legitimately
/// hand-builds compositions through `Engine::compose_modules`, and a widened
/// tripwire would trip on a test that is doing the right thing.
#[test]
fn the_composition_root_is_stated_once_across_the_fork() {
    const FILES: [(&str, &str); 4] = [
        (
            "crates/mush-server/src/boot.rs",
            include_str!("../src/boot.rs"),
        ),
        (
            "crates/mush-server/src/lib.rs",
            include_str!("../src/lib.rs"),
        ),
        (
            "crates/mushd/src/boot.rs",
            include_str!("../../mushd/src/boot.rs"),
        ),
        (
            "crates/mushd/src/main.rs",
            include_str!("../../mushd/src/main.rs"),
        ),
    ];
    for needle in ["Engine::compose_modules(", "panel_feeds()"] {
        let sites: Vec<String> = FILES
            .iter()
            .filter(|(_, src)| src.contains(needle))
            .map(|(name, src)| format!("{name} x{}", src.matches(needle).count()))
            .collect();
        let total: usize = FILES
            .iter()
            .map(|(_, src)| src.matches(needle).count())
            .sum();
        assert_eq!(
            total, 1,
            "`{needle}` must be stated exactly once across the fork's four boot \
             files, in crates/mush-server/src/boot.rs; found it at {sites:?}. A \
             second site is the world boot being transcribed again, and the two \
             will drift by omission exactly as they did before WS04-S0"
        );
        assert_eq!(
            sites,
            ["crates/mush-server/src/boot.rs x1"],
            "`{needle}`'s one home is the shared root"
        );
    }
}
