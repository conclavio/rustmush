//! M3 module wiring: which [`GameModule`]s the server boots with, their
//! persisted-data load path, and the per-namespace module migrations.
//!
//! Boot order (driven from `main.rs` / test rigs via [`boot_modules`]):
//!
//! 1. [`Engine::with_modules`] has already run every module's `register`
//!    (commands, hooks, ticks, state slots) against the one registry.
//! 2. [`run_module_migrations`] brings each registered namespace's
//!    persisted documents up to its module's current schema (BGSAVE first —
//!    the mandatory pre-migrate snapshot), **before** any module parses
//!    them.
//! 3. [`load_module_boot`] reads each module's namespace through the store
//!    read API — every kv doc (minus the reserved [`SCHEMA_KEY`]) plus the
//!    streams the module names via
//!    [`GameModule::boot_stream_requests`].
//! 4. [`Engine::modules_loaded`] hands each module its
//!    [`ModuleBootData`]; whatever the modules persist during boot
//!    (first-boot seeding, orphan-doc cleanup) is drained and written
//!    through **durably, acked** before the server starts serving.
//!
//! The module list itself is [`server_modules`]: comsys first (its
//! [`AliasStage`](mush_core::AliasStage) is the earliest alias stage in
//! dispatch), then mail, then jobs, then weather, then script (both M5).
//! Their persistence namespaces are disjoint, so the order is otherwise
//! cosmetic — but it is the registration order hooks and observers fire in,
//! so keep it stable.
//!
//! Script goes **last** deliberately. Its object `$`-command matcher is a
//! post-command-table [`CommandFallback`](mush_core::CommandFallback), not an
//! alias stage, so it never competes with comsys for dispatch order. Running
//! its `register` after the others means its `on_world_loaded` snapshots a
//! registry that already carries every other module's attr namespaces (the
//! off-loop session enforces module-namespace ownership against that
//! snapshot), and its `AttrSet`/`Moved`/`Said` observers fire after the
//! game-system modules' observers for the same event — the intended order,
//! since a scripted reflex should see the world the built-ins already left.
//!
//! The M0 `--repl` stays module-free on purpose (offline poking of the core
//! surface; it has no store to load module data from).

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{Effects, Engine, GameModule, Migration, ModuleBootData, Outbox};
use mush_llm::LlmService;
use mush_store::{ModuleOp, ModuleWrite, ObjectStore, WriteBatch};
use mush_worldgen::WorldgenModule;
use smol_str::SmolStr;

/// Reserved kv key per module namespace carrying `{"version": N}` — the
/// namespace's schema version for [`run_module_migrations`]. Filtered out
/// of [`ModuleBootData::kv`] so modules never see it.
pub const SCHEMA_KEY: &str = "schema";

/// The built-in game modules a production `mushd` boots with, in
/// registration order. Config-selected [`ruleset_modules`] are appended
/// **after** these (see `main.rs`); hand the concatenation to
/// [`Engine::with_modules`].
#[must_use]
pub fn server_modules() -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_comsys::ComsysModule::new()),
        Box::new(mush_mail::MailModule),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(mush_weather::WeatherModule::new()),
        Box::new(mush_script::ScriptModule::new()),
    ]
}

/// The ruleset module(s) named in `[rulesets].enabled`, in listed order.
///
/// Every ruleset crate is compiled into `mushd`; this maps the configured
/// ids to their modules. Registered **after** [`server_modules`] so a
/// ruleset's commands, panel feeds, and job queues wire cleanly and its
/// approval payload handlers (registered in `on_world_loaded`) run after the
/// jobs module's state slot exists.
///
/// A ruleset's sheets ride the object attr write-through (treaty ruling 7),
/// so they load with the world — a ruleset registers no boot namespace and
/// needs no separate boot-data load here.
///
/// An unknown id is a hard error (consistent with the rest of config: a
/// silently-ignored typo is worse than not starting). The only id available
/// in M6a is `"wod"` (oWoD20); duplicates are rejected too.
pub fn ruleset_modules(enabled: &[String]) -> Result<Vec<Box<dyn GameModule>>, String> {
    let mut seen: Vec<String> = Vec::new();
    let mut mods: Vec<Box<dyn GameModule>> = Vec::new();
    for id in enabled {
        let key = id.trim().to_lowercase();
        if seen.contains(&key) {
            return Err(format!("[rulesets].enabled lists {id:?} more than once"));
        }
        match key.as_str() {
            "wod" => mods.push(Box::new(ruleset_wod::WodRuleset::new())),
            _ => {
                return Err(format!(
                    "[rulesets].enabled: unknown ruleset {id:?} (available: \"wod\")"
                ));
            }
        }
        seen.push(key);
    }
    Ok(mods)
}

/// The M7 worldgen module, backed by the shared [`LlmService`] handle.
///
/// `main.rs` inserts it into the boot list **after** [`server_modules`] (so it
/// registers after the built-ins) and before the config-selected
/// [`ruleset_modules`]. It registers regardless of whether the LLM is enabled:
/// with `[llm].enabled = false` the service's kill switch is engaged, so
/// `@gen` still parses and defers, and the off-loop generator re-enters a
/// "disabled" outcome (a keyless clone boots and its worldgen commands answer
/// cleanly rather than 404). Its persistence namespace (`worldgen`) is disjoint
/// from every other module's, and it loads its drafts through the same
/// boot-data path as the rest ([`load_module_boot`]).
#[must_use]
pub fn worldgen_module(llm: Arc<LlmService>) -> Box<dyn GameModule> {
    Box::new(WorldgenModule::new(llm))
}

/// The M8 NPC agent module. Registered after worldgen and before the rulesets.
/// `grounding` supplies the ruleset sheet summary + ability tools (the WoD
/// adapter when `wod` is enabled); `None` runs NPCs dialogue-only.
#[must_use]
pub fn npc_module(grounding: Option<Arc<dyn mush_npc::AgentGrounding>>) -> Box<dyn GameModule> {
    match grounding {
        Some(g) => Box::new(mush_npc::NpcModule::with_grounding(g)),
        None => Box::new(mush_npc::NpcModule::new()),
    }
}

/// Build the NPC↔ruleset grounding adapter for the enabled rulesets, if any
/// grounds NPC agents. `main.rs` passes it to [`npc_module`]. Only `wod` grounds
/// agents in M8; a config without it leaves NPCs dialogue-only.
#[must_use]
pub fn npc_agent_adapter(enabled: &[String]) -> Option<Arc<WodAgent>> {
    if enabled.iter().any(|s| s.trim().eq_ignore_ascii_case("wod")) {
        Some(Arc::new(WodAgent {
            wod: ruleset_wod::WodRuleset::new(),
        }))
    } else {
        None
    }
}

/// The WoD grounding adapter (M8), holding its own [`WodRuleset`]. Both halves
/// of [`AgentGrounding`](mush_npc::AgentGrounding) run **on the world loop**:
/// `profile` reads the NPC's `sheet:wod` document and grounds it via
/// [`Ruleset::agent_profile`](mush_ruleset::Ruleset::agent_profile);
/// `invoke_ability` executes a `wod_roll` (etc.) via
/// [`Ruleset::invoke_ability`](mush_ruleset::Ruleset::invoke_ability), which
/// reads the NPC's own sheet, rolls on the shared RNG, and emits to the room.
pub struct WodAgent {
    wod: ruleset_wod::WodRuleset,
}

impl mush_npc::AgentGrounding for WodAgent {
    fn profile(
        &self,
        w: &mush_core::World,
        npc: mush_core::ObjId,
    ) -> Option<mush_ruleset::AgentCapabilityProfile> {
        use mush_ruleset::Ruleset;
        let doc = self.wod.store().get(w, npc)?;
        Some(self.wod.agent_profile(&doc))
    }

    fn invoke_ability(
        &self,
        w: &mut mush_core::World,
        out: &mut mush_core::Outbox,
        npc: mush_core::ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<mush_ruleset::AbilityOutcome, mush_ruleset::AbilityError> {
        use mush_ruleset::Ruleset;
        self.wod.invoke_ability(w, out, npc, name, input)
    }

    fn init_sheet(
        &self,
        w: &mut mush_core::World,
        npc: mush_core::ObjId,
        template: &str,
        by: mush_core::ObjId,
    ) -> Result<String, String> {
        // The `@agent/sheet` bootstrap: give the NPC a real WoD sheet through
        // the ruleset's validated retemplate path so `wod_roll` has traits.
        self.wod.init_sheet(w, npc, template, by)
    }
}

/// Read one module's persisted boot data: every kv doc in its namespace
/// (minus the reserved [`SCHEMA_KEY`]) plus the streams the module asks
/// for given those docs.
async fn load_one(
    store: &dyn ObjectStore,
    module: &dyn GameModule,
) -> Result<ModuleBootData, String> {
    let ns = module.name();
    let kv: Vec<(SmolStr, serde_json::Value)> = store
        .module_json_list(ns)
        .await
        .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?
        .into_iter()
        .filter(|(k, _)| k != SCHEMA_KEY)
        .map(|(k, v)| (SmolStr::new(k), v))
        .collect();
    let mut streams = HashMap::new();
    for (key, count) in module.boot_stream_requests(&kv) {
        let entries = store
            .stream_range(ns, &key, count)
            .await
            .map_err(|e| format!("loading module stream {ns}:{key}: {e}"))?;
        streams.insert(key, entries);
    }
    Ok(ModuleBootData { kv, streams })
}

/// Load every registered module's persisted boot data, keyed by module
/// name, ready for [`Engine::modules_loaded`].
pub async fn load_module_boot(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
) -> Result<HashMap<SmolStr, ModuleBootData>, String> {
    let mut boot = HashMap::new();
    for m in modules {
        boot.insert(SmolStr::new(m.name()), load_one(store, m.as_ref()).await?);
    }
    Ok(boot)
}

/// The schema version a namespace's [`SCHEMA_KEY`] doc claims, if any.
async fn ns_version(store: &dyn ObjectStore, ns: &str) -> Result<Option<u32>, String> {
    let doc = store
        .module_json_get(ns, SCHEMA_KEY)
        .await
        .map_err(|e| format!("reading {ns} schema doc: {e}"))?;
    Ok(doc
        .and_then(|d| d.get("version").and_then(serde_json::Value::as_u64))
        .map(|v| u32::try_from(v).unwrap_or(u32::MAX)))
}

/// Write one batch of module ops through the store, ack awaited (boot runs
/// before the world task, so it writes directly).
async fn write_ops(store: &dyn ObjectStore, ops: Vec<ModuleOp>) -> Result<(), String> {
    let batch = WriteBatch {
        serial: 0,
        durable: true,
        module_ops: ops,
        ..WriteBatch::default()
    };
    store
        .write(batch)
        .await
        .map(|_| ())
        .map_err(|e| format!("module boot write: {e}"))
}

/// A `JsonSet` stamping a namespace's schema version.
fn stamp(ns: &str, version: u32) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new(ns),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new(SCHEMA_KEY),
            value: serde_json::json!({ "version": version }),
        },
    }
}

/// Run every module's registered per-namespace migrations (collected via
/// `Registrar::migrations`, exposed by
/// [`Registry::migrations`](mush_core::Registry::migrations)) against the
/// store, following the same rules as the core schema
/// ([`mush_store::run_migrations`]):
///
/// - a **fresh namespace** (no schema doc, no documents) is stamped to the
///   latest registered version without running anything;
/// - an unstamped namespace **with** documents is pre-versioning data at
///   v0: every migration runs;
/// - a namespace **newer than the build** refuses to boot;
/// - before any migration actually runs, `bgsave` is triggered once (the
///   mandatory pre-migrate snapshot);
/// - each step transforms every document in the namespace (minus the
///   schema doc) and commits them **with** the version bump in one batch,
///   so a crash between steps resumes exactly where it stopped.
pub async fn run_module_migrations(
    store: &dyn ObjectStore,
    migrations: &HashMap<SmolStr, Vec<Migration>>,
) -> Result<(), String> {
    let mut namespaces: Vec<&SmolStr> = migrations.keys().collect();
    namespaces.sort(); // deterministic order across boots
    let mut snapshotted = false;
    for ns in namespaces {
        let migs = &migrations[ns];
        let Some(latest) = migs.last().map(|m| m.version) else {
            continue;
        };
        let mut prev = 0u32;
        for m in migs {
            if m.version <= prev {
                return Err(format!(
                    "module {ns}: migration set not strictly ascending at v{}",
                    m.version
                ));
            }
            prev = m.version;
        }
        let current = match ns_version(store, ns).await? {
            Some(v) => v,
            None => {
                let docs = store
                    .module_json_list(ns)
                    .await
                    .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
                if docs.is_empty() {
                    // Fresh namespace: stamp to latest, run nothing.
                    write_ops(store, vec![stamp(ns, latest)]).await?;
                    tracing::info!(ns = %ns, version = latest, "fresh module namespace stamped");
                    continue;
                }
                0 // pre-versioning data
            }
        };
        if current > latest {
            return Err(format!(
                "module {ns}: stored schema v{current} is newer than this build's \
                 v{latest}; refusing to run (restore from backup to downgrade)"
            ));
        }
        if current == latest {
            continue;
        }
        if !snapshotted {
            store
                .bgsave()
                .await
                .map_err(|e| format!("pre-migrate BGSAVE failed: {e}"))?;
            snapshotted = true;
        }
        for m in migs.iter().filter(|m| m.version > current) {
            tracing::info!(ns = %ns, to = m.version, desc = %m.description,
                "running module migration");
            let docs = store
                .module_json_list(ns)
                .await
                .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
            let mut ops = Vec::new();
            for (key, value) in docs {
                if key == SCHEMA_KEY {
                    continue;
                }
                let migrated = (m.apply)(&key, value).map_err(|e| {
                    format!("module {ns} migration to v{}: doc {key:?}: {e}", m.version)
                })?;
                ops.push(ModuleOp {
                    ns: ns.clone(),
                    op: ModuleWrite::JsonSet {
                        key: SmolStr::new(key),
                        value: migrated,
                    },
                });
            }
            // The version bump commits with the transformed docs: a crash
            // between steps re-runs from exactly the stored version.
            ops.push(stamp(ns, m.version));
            write_ops(store, ops).await?;
        }
    }
    Ok(())
}

/// The full module boot sequence over an already-constructed engine (see
/// the module docs): migrations → boot data → `modules_loaded` → durable
/// write-through of whatever boot persisted. Call between
/// [`Engine::with_modules`] and spawning the world task.
pub async fn boot_modules(engine: &mut Engine, store: &dyn ObjectStore) -> Result<(), String> {
    run_module_migrations(store, engine.registry.migrations()).await?;
    let boot = load_module_boot(store, engine.modules()).await?;
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine
        .modules_loaded(boot, &mut out, &mut fx)
        .map_err(|e| format!("module boot failed: {e}"))?;
    // Nothing is connected yet; boot-time output has nowhere to go.
    let events = out.take();
    if !events.is_empty() {
        tracing::debug!(
            count = events.len(),
            "module boot output dropped (no sessions yet)"
        );
    }
    debug_assert!(fx.is_empty(), "modules must not defer effects at boot");
    // First-boot seeding / orphan cleanup must be durable before serving.
    let ops = engine.take_module_ops();
    if !ops.is_empty() {
        write_ops(store, ops).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use mush_core::{ModError, Registrar, World};
    use mush_store::InMemoryStore;

    /// A module that persists one doc under ns "toy" and registers a v1→v2
    /// migration renaming field `a` to `b`.
    struct ToyMod {
        loaded: Vec<(SmolStr, serde_json::Value)>,
    }

    impl GameModule for ToyMod {
        fn name(&self) -> &'static str {
            "toy"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.migrations(
                "toy",
                vec![
                    Migration {
                        version: 1,
                        description: "initial".into(),
                        apply: Arc::new(|_key, v| Ok(v)),
                    },
                    Migration {
                        version: 2,
                        description: "rename a to b on doc:* only".into(),
                        // The key parameter routes heterogeneous
                        // namespaces: only doc:* documents are reshaped.
                        apply: Arc::new(|key: &str, mut v: serde_json::Value| {
                            if key.starts_with("doc:")
                                && let Some(obj) = v.as_object_mut()
                                && let Some(a) = obj.remove("a")
                            {
                                obj.insert("b".into(), a);
                            }
                            Ok(v)
                        }),
                    },
                ],
            )?;
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            self.loaded = data.kv;
            Ok(())
        }
    }

    fn toy_engine() -> Engine {
        Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(ToyMod { loaded: Vec::new() })],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn fresh_namespace_is_stamped_without_running() {
        let store = InMemoryStore::new();
        let mut engine = toy_engine();
        boot_modules(&mut engine, &store).await.unwrap();
        let v = ns_version(&store, "toy").await.unwrap();
        assert_eq!(v, Some(2));
    }

    #[tokio::test]
    async fn preversioned_docs_migrate_and_stamp() {
        let store = InMemoryStore::new();
        // Seed a pre-versioning doc (no schema key).
        write_ops(
            &store,
            vec![ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: "doc:1".into(),
                    value: serde_json::json!({ "a": 42 }),
                },
            }],
        )
        .await
        .unwrap();

        let mut engine = toy_engine();
        boot_modules(&mut engine, &store).await.unwrap();

        let doc = store
            .module_json_get("toy", "doc:1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc, serde_json::json!({ "b": 42 }));
        assert_eq!(ns_version(&store, "toy").await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn newer_store_refuses_to_boot() {
        let store = InMemoryStore::new();
        write_ops(&store, vec![stamp("toy", 9)]).await.unwrap();
        let mut engine = toy_engine();
        let err = boot_modules(&mut engine, &store).await.unwrap_err();
        assert!(err.contains("newer than this build"), "{err}");
    }

    #[tokio::test]
    async fn schema_key_never_reaches_module_boot_data() {
        let store = InMemoryStore::new();
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc:1".into(),
                        value: serde_json::json!({ "b": 1 }),
                    },
                },
            ],
        )
        .await
        .unwrap();
        let modules: Vec<Box<dyn GameModule>> = vec![Box::new(ToyMod { loaded: Vec::new() })];
        let boot = load_module_boot(&store, &modules).await.unwrap();
        let kv = &boot[&SmolStr::new("toy")].kv;
        assert_eq!(kv.len(), 1);
        assert_eq!(kv[0].0, "doc:1");
    }

    #[tokio::test]
    async fn server_modules_boot_and_seed_durably() {
        // The real module list boots against an empty store; comsys'
        // first-boot seeding (the Public channel) must be written through.
        let store = InMemoryStore::new();
        let mut registry = crate::world_task::base_registry();
        let staff = std::sync::Arc::new(crate::staff::StaffQueue::new());
        crate::staff::register_server_commands(&mut registry, &staff).unwrap();
        let mut engine =
            Engine::with_modules(World::new_minimal(), registry, server_modules()).unwrap();
        boot_modules(&mut engine, &store).await.unwrap();

        let chans = store.module_json_list("comsys").await.unwrap();
        assert!(
            chans.iter().any(|(k, v)| k.starts_with("chan:")
                && v.get("name").and_then(serde_json::Value::as_str) == Some("Public")),
            "first boot must seed the Public channel durably: {chans:?}"
        );

        // A second boot over the same store must NOT re-seed (the member
        // docs and channel survive; God's membership from boot one holds).
        let mut registry = crate::world_task::base_registry();
        crate::staff::register_server_commands(&mut registry, &staff).unwrap();
        let mut engine2 =
            Engine::with_modules(World::new_minimal(), registry, server_modules()).unwrap();
        boot_modules(&mut engine2, &store).await.unwrap();
        let chans2 = store.module_json_list("comsys").await.unwrap();
        let count = |cs: &[(String, serde_json::Value)]| {
            cs.iter().filter(|(k, _)| k.starts_with("chan:")).count()
        };
        assert_eq!(count(&chans), count(&chans2), "reboot must not re-seed");
    }

    #[test]
    fn m5_modules_opt_into_the_testkit_cleanly() {
        // The testkit's `with_modules` is the module-authoring on-ramp: the
        // M5 scripting + weather modules must register (commands, hooks,
        // state slots), first-boot, and surface their seams there — the
        // "opt into the modules" confirmation for both.
        use mush_testkit::TestWorld;
        let tw = TestWorld::with_modules(vec![
            Box::new(mush_weather::WeatherModule::new()),
            Box::new(mush_script::ScriptModule::new()),
        ]);
        // Weather filled its reserved query seam...
        let (owner, _) = tw.registry().weather_query().expect("weather query seam");
        assert_eq!(owner.as_str(), "weather");
        // ...and script installed its post-command-table `$`-command
        // fallback (the object-command matcher).
        assert!(
            tw.registry().command_fallback_count() >= 1,
            "script registers a command fallback"
        );
        // Both modules are present under the one registry, no collision.
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert!(
            names.contains(&"weather") && names.contains(&"script"),
            "{names:?}"
        );
    }

    #[test]
    fn server_modules_register_without_collision() {
        // The full production list (comsys, mail, jobs, weather, script)
        // shares one registry: registration must not collide, and every
        // module must be present in boot order.
        use mush_testkit::TestWorld;
        let tw = TestWorld::with_modules(server_modules());
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(names, ["comsys", "mail", "jobs", "weather", "script"]);
    }

    #[test]
    fn ruleset_modules_selects_wod() {
        // "wod" resolves to the WoD ruleset module; the id survives boot.
        let mods = ruleset_modules(&["wod".to_string()]).unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name(), "wod");
        // Case/whitespace-insensitive.
        let mods = ruleset_modules(&["  WoD ".to_string()]).unwrap();
        assert_eq!(mods[0].name(), "wod");
        // Empty list is fine (a ruleset-less server).
        assert!(ruleset_modules(&[]).unwrap().is_empty());
    }

    #[test]
    fn ruleset_modules_rejects_unknown_and_duplicate() {
        // `unwrap_err` needs `Debug` on the Ok value; `dyn GameModule` has
        // none, so match the error out by hand.
        let err = match ruleset_modules(&["btech".to_string()]) {
            Err(e) => e,
            Ok(_) => panic!("unknown ruleset must be rejected"),
        };
        assert!(err.contains("unknown ruleset"), "{err}");
        let err = match ruleset_modules(&["wod".to_string(), "wod".to_string()]) {
            Err(e) => e,
            Ok(_) => panic!("duplicate ruleset must be rejected"),
        };
        assert!(err.contains("more than once"), "{err}");
    }

    #[test]
    fn worldgen_registers_in_the_full_boot_list() {
        // main.rs's assembly: built-ins THEN worldgen THEN the config-selected
        // ruleset, sharing one registry. Worldgen registers with a
        // MockProvider-backed service (no network, no key); its @gen/@theme/@llm
        // commands and the worldgen.draft panel feed must all wire in cleanly.
        use mush_llm::{LlmService, MockProvider};
        use mush_testkit::TestWorld;

        let mock: Arc<dyn mush_llm::LlmProvider> = Arc::new(MockProvider::new("openrouter"));
        let llm = Arc::new(
            LlmService::builder()
                .provider("openrouter", mock)
                .global_enabled(false)
                .build(),
        );
        let mut mods = server_modules();
        mods.push(worldgen_module(Arc::clone(&llm)));
        mods.extend(ruleset_modules(&["wod".to_string()]).unwrap());
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys", "mail", "jobs", "weather", "script", "worldgen", "wod"
            ]
        );
        assert!(tw.registry().spec_of("@gen").is_some());
        assert!(tw.registry().spec_of("@theme").is_some());
        assert!(tw.registry().spec_of("@llm").is_some());
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "worldgen.draft")
        );
    }

    #[test]
    fn npc_module_registers_and_joins_the_panel_manifest() {
        // main.rs builds the web hello manifest from `registry.panel_feeds()`,
        // so registering `npc.thinking` is what puts the thinking indicator in
        // the manifest. This guards that (and that the npc module registers
        // cleanly alongside the built-ins). Dialogue-only grounding is enough.
        use mush_testkit::TestWorld;
        let mut mods = server_modules();
        mods.push(npc_module(None));
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert!(names.contains(&"npc"), "{names:?}");
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "npc.thinking"),
            "npc.thinking must be a registered panel feed (it joins the web manifest)"
        );
        // The @agent staff surface is live.
        assert!(tw.registry().spec_of("@agent").is_some());
    }

    #[test]
    fn npc_agent_adapter_only_grounds_when_wod_enabled() {
        // The WoD grounding adapter is built only when "wod" is in the enabled
        // rulesets; otherwise NPCs run dialogue-only (None).
        assert!(npc_agent_adapter(&["wod".to_string()]).is_some());
        assert!(npc_agent_adapter(&["  WoD ".to_string()]).is_some());
        assert!(npc_agent_adapter(&[]).is_none());
        assert!(npc_agent_adapter(&["btech".to_string()]).is_none());
    }

    #[test]
    fn full_boot_list_registers_without_collision() {
        // The production concatenation main.rs builds — built-ins THEN the
        // config-selected wod ruleset — shares one registry with no
        // collision, and wod registers last (after jobs, so its approval
        // payload handlers see a loaded jobs module).
        use mush_testkit::TestWorld;
        let mut mods = server_modules();
        mods.extend(ruleset_modules(&["wod".to_string()]).unwrap());
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            ["comsys", "mail", "jobs", "weather", "script", "wod"]
        );
        // The ruleset's surface is live: +roll / +sheet registered, the
        // wod.roll panel feed present, and the chargen/xp job queues too.
        assert!(tw.registry().spec_of("+roll").is_some());
        assert!(tw.registry().spec_of("+sheet").is_some());
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "wod.roll")
        );
    }
}
