//! The server's view of the composition root, which lives in
//! [`tacsim::composition`] since 16b cut R-a (R2). Everything the host and its
//! tests reach as `mush_server::modules::*` is re-exported here unchanged; the
//! composition tests below stay in this crate because they drive the host's own
//! boot, persistence and staff seams.

pub use tacsim::composition::*;

use std::collections::HashMap;

use mush_core::{
    ActionStateBootRecord, BOOT_MANIFEST_KEY, BootManifest, Effects, Engine, GameModule, Migration,
    ModError, ModuleBootData, Outbox, is_action_state_persistence_key,
};
#[cfg(test)]
use mush_llm::LlmService;
use mush_store::{ModuleOp, ModuleWrite, ObjectStore, WriteBatch};
use smol_str::SmolStr;

/// What a namespace's boot manifest lets the loader skip (the chunk floors
/// and the key prefixes, doc 13 W1), or nothing: no manifest, an unreadable
/// one (said at `warn`), or a format this loader does not know.
async fn manifest_skip(store: &dyn ObjectStore, ns: &str) -> Result<mush_store::BootSkip, String> {
    let Some(doc) = store
        .module_json_get(ns, BOOT_MANIFEST_KEY)
        .await
        .map_err(|e| format!("reading {ns} boot manifest: {e}"))?
    else {
        return Ok(mush_store::BootSkip::default());
    };
    match serde_json::from_value::<BootManifest>(doc) {
        Ok(manifest) => {
            if manifest.format != mush_core::BOOT_MANIFEST_FORMAT {
                tracing::warn!(
                    ns,
                    format = manifest.format,
                    "boot manifest at an unknown format; fetching the namespace whole"
                );
            }
            Ok(mush_store::BootSkip {
                floors: manifest
                    .floors()
                    .into_iter()
                    .map(|(log, below)| (log.to_string(), below))
                    .collect(),
                prefixes: manifest
                    .prefixes()
                    .into_iter()
                    .map(|p| p.to_string())
                    .collect(),
            })
        }
        Err(error) => {
            tracing::warn!(ns, %error, "boot manifest unreadable; fetching the namespace whole");
            Ok(mush_store::BootSkip::default())
        }
    }
}

async fn load_one_namespace(
    store: &dyn ObjectStore,
    ns: &str,
    fetch: Fetch,
) -> Result<(ModuleBootData, Vec<ActionStateBootRecord>), String> {
    // J1 / doc 12 D-12-3: the fetch is timed and handed to the module beside
    // its data — the part of a boot the replay timer never saw.
    let started = std::time::Instant::now();
    // J4: the manifest first, then a listing that never fetches what it names.
    // A whole fetch (the retry) still honours the manifest's key prefixes
    // (doc 13 W1): those documents are read on demand and never at boot,
    // whatever the checkpoint's state.
    let mut skip = manifest_skip(store, ns).await?;
    if fetch == Fetch::Whole {
        skip.floors.clear();
    }
    let documents = store
        .module_docs_from(ns, &skip)
        .await
        .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
    let fetch_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut kv = Vec::new();
    let mut action_state = Vec::new();
    for (key, value) in documents.docs {
        // Loader-owned protocol documents never reach the module.
        if key == SCHEMA_KEY || key == BOOT_MANIFEST_KEY {
            continue;
        }
        if is_action_state_persistence_key(&key) {
            action_state.push(ActionStateBootRecord::new(ns, key, value));
        } else {
            kv.push((SmolStr::new(key), value));
        }
    }
    let ledger = documents
        .ledger
        .into_iter()
        .map(|(key, text)| (SmolStr::new(key), text))
        .collect();
    let omitted = documents
        .omitted
        .into_iter()
        .map(|(log, below)| (SmolStr::new(log), below))
        .collect();
    if !skip.is_empty() {
        tracing::info!(
            ns,
            floors = ?skip.floors,
            prefixes = ?skip.prefixes,
            fetch_ms,
            "namespace fetched narrowed to its boot manifest"
        );
    }
    Ok((
        ModuleBootData {
            kv,
            ledger,
            streams: HashMap::new(),
            fetch_ms,
            omitted,
        },
        action_state,
    ))
}

/// Only after typed state has hydrated, let modules select their bounded
/// stream tails from already-partitioned ordinary JSON data.
async fn load_requested_streams(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
    boot: &mut HashMap<SmolStr, ModuleBootData>,
) -> Result<(), String> {
    for module in modules {
        let Some(data) = boot.get_mut(module.name()) else {
            return Err(format!(
                "module namespace {:?} was not loaded in the first pass",
                module.name()
            ));
        };
        load_streams_for(store, module.as_ref(), data).await?;
    }
    Ok(())
}

/// [`load_requested_streams`] for one module's data (the J4 retry refetches
/// one namespace and needs its streams again).
async fn load_streams_for(
    store: &dyn ObjectStore,
    module: &dyn GameModule,
    data: &mut ModuleBootData,
) -> Result<(), String> {
    let ns = module.name();
    let mut streams = HashMap::new();
    let stream_requests = module
        .boot_stream_requests(&data.kv)
        .map_err(|e| format!("validating module stream requests for {ns:?}: {e}"))?;
    for (key, count) in stream_requests {
        let entries = store
            .stream_range(ns, &key, count)
            .await
            .map_err(|e| format!("loading module stream {ns}:{key}: {e}"))?;
        streams.insert(key, entries);
    }
    data.streams = streams;
    Ok(())
}

/// Load only ordinary module boot data. The full public boot path is
/// [`boot_modules`], which also hydrates the reserved typed-state partition.
#[cfg(test)]
async fn load_ordinary_module_boot(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
) -> Result<HashMap<SmolStr, ModuleBootData>, String> {
    let (mut boot, _) = load_module_boot(store, modules).await?;
    load_requested_streams(store, modules, &mut boot).await?;
    Ok(boot)
}

async fn load_module_boot(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
) -> Result<(HashMap<SmolStr, ModuleBootData>, Vec<ActionStateBootRecord>), String> {
    let mut boot = HashMap::new();
    let mut action_state = Vec::new();
    for module in modules {
        let (module_boot, mut module_action_state) =
            load_one_namespace(store, module.name(), Fetch::Narrowed).await?;
        boot.insert(SmolStr::new(module.name()), module_boot);
        action_state.append(&mut module_action_state);
    }
    Ok((boot, action_state))
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
        serial: BOOT_SERIAL,
        durable: true,
        module_ops: ops,
        ..WriteBatch::default()
    };
    write_batch(store, batch).await
}

/// Commit one boot-time batch, durably, checking the receipt's serial.
async fn write_batch(store: &dyn ObjectStore, batch: WriteBatch) -> Result<(), String> {
    let receipt = store
        .write(batch)
        .await
        .map_err(|e| format!("module boot write: {e}"))?;
    if receipt.serial != BOOT_SERIAL {
        return Err(format!(
            "module boot write: expected receipt serial {BOOT_SERIAL}, got {}",
            receipt.serial
        ));
    }
    Ok(())
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
                if docs
                    .iter()
                    .all(|(key, _)| key == SCHEMA_KEY || is_action_state_persistence_key(key))
                {
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
                if key == SCHEMA_KEY || is_action_state_persistence_key(&key) {
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
/// the module docs): migrations → boot data → `module_loaded` per module →
/// `all_modules_loaded` (every `on_all_loaded`, then the census) → durable
/// write-through of whatever boot persisted. Call between
/// [`Engine::from_composition`] (or the compatibility
/// [`Engine::with_modules`] wrapper) and spawning the world task.
pub async fn boot_modules(engine: &mut Engine, store: &dyn ObjectStore) -> Result<(), String> {
    run_module_migrations(store, engine.registry.migrations()).await?;
    let (mut boot, action_state) = load_module_boot(store, engine.modules()).await?;
    engine
        .hydrate_action_state(action_state)
        .map_err(|error| format!("action-state boot failed: {error}"))?;
    load_requested_streams(store, engine.modules(), &mut boot).await?;
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    // J4 (doc 12 §5.6): the modules are delivered one at a time, in order, so
    // a module that cannot boot from a namespace narrowed by its own manifest
    // (the checkpoint it stood for proved invalid or gone) gets that namespace
    // fetched whole — manifest ignored — and delivered once more, itself
    // alone; the retry's failure is the boot's.
    let names: Vec<SmolStr> = engine
        .modules()
        .iter()
        .map(|m| SmolStr::new(m.name()))
        .collect();
    for name in names {
        let data = boot.remove(&name).unwrap_or_default();
        match engine.module_loaded(&name, data, &mut out, &mut fx) {
            Ok(()) => {}
            Err(ModError::RetryFullFetch { reason, .. }) => {
                tracing::warn!(
                    module = %name,
                    %reason,
                    "module cannot boot from its narrowed namespace; fetching it whole and delivering again"
                );
                // The manifest is loader-owned and its floors stood for a
                // checkpoint that did not hold: clear the floors, so the next
                // boot fetches the ledger whole until the module writes a new
                // manifest beside its next checkpoint. The key prefixes stay
                // (doc 13 W1): they name documents the module never loads at
                // boot.
                let kept = manifest_skip(store, &name).await?;
                let cleared = BootManifest {
                    format: mush_core::BOOT_MANIFEST_FORMAT,
                    skip_log_chunks_below: std::collections::BTreeMap::new(),
                    skip_key_prefixes: kept.prefixes.iter().map(SmolStr::new).collect(),
                };
                write_ops(
                    store,
                    vec![ModuleOp {
                        ns: name.clone(),
                        op: ModuleWrite::JsonSet {
                            key: SmolStr::new(BOOT_MANIFEST_KEY),
                            value: serde_json::to_value(&cleared)
                                .expect("a boot manifest serializes"),
                        },
                    }],
                )
                .await?;
                let (mut data, _already_hydrated_action_state) =
                    load_one_namespace(store, &name, Fetch::Whole).await?;
                let module = engine
                    .modules()
                    .iter()
                    .find(|m| m.name() == name)
                    .expect("the module was just delivered");
                load_streams_for(store, module.as_ref(), &mut data).await?;
                engine
                    .module_loaded(&name, data, &mut out, &mut fx)
                    .map_err(|e| format!("module boot failed after the whole fetch: {e}"))?;
            }
            Err(e) => return Err(format!("module boot failed: {e}")),
        }
    }
    // 16b cut U.4 (16c `B-37`): every module is loaded, so every module's
    // `on_all_loaded` runs now, in list order — and then the WS03-S8 census,
    // since every slot that will exist exists. This root delivers the modules
    // one at a time (the J4 retry needs it), so it ends the boot itself —
    // `Engine::modules_loaded`, the all-at-once path, calls the same
    // `Engine::all_modules_loaded` for its own callers.
    engine
        .all_modules_loaded(&mut out, &mut fx)
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
    if let Some((namespace, key)) = ops.iter().find_map(|op| {
        let key = match &op.op {
            ModuleWrite::JsonSet { key, .. }
            | ModuleWrite::Del { key }
            | ModuleWrite::StreamAppend { key, .. } => key,
        };
        is_action_state_persistence_key(key).then_some((op.ns.as_str(), key.as_str()))
    }) {
        return Err(format!(
            "module boot attempted coordinator-reserved persistence key {namespace}:{key}"
        ));
    }
    // S1: a module boot pass may also make OBJECTS — the content module applies the
    // starter packs, which dig rooms and place NPCs — and those ride the SAME batch
    // as the ops, so the content latch and the objects it describes commit together
    // or not at all ("objects and the latch or neither"). A crash between two writes
    // would otherwise leave a latch that says the lot exists over a world without
    // it, and the next boot would believe the latch.
    let dirty = engine.world.take_dirty();
    if ops.is_empty() && dirty.is_empty() {
        return Ok(());
    }
    let mut batch = crate::persist::batch_from_dirty(&engine.world, dirty, BOOT_SERIAL, true);
    batch.module_ops = ops;
    write_batch(store, batch).await
}

/// The serial every boot-time batch carries: the world seed's, the module boot
/// write's, and the manifest reset's. Receipts echo it; the world task's own serials
/// start after boot.
const BOOT_SERIAL: u64 = 0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// S6 (doc 14 §4.6): the game's sheet composes its rules interface beside it,
    /// carrying the chargen stages.
    ///
    /// 16b cut R-b: there is no ruleset SELECTION left to prove — `sheet_modules`
    /// is infallible and always answers the pilot sheet — so what this pins is the
    /// PAIRING and its order. The interface snapshots the ruleset's stages at
    /// composition, so it has to come second, and it has to carry the stages the
    /// sheet's own chargen declares rather than an empty list.
    #[test]
    fn a_ruleset_brings_its_rules_interface() {
        let mods = sheet_modules(
            &mush_core::OnboardingConfig::default(),
            &tacsim::GameEconomyConfig::default(),
        );
        let names: Vec<&str> = mods.iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                tacsim::sheet::NS,
                mush_ruleset::interface::RULES_INTERFACE_MODULE
            ],
            "the pilot sheet, and the interface that fronts its chargen stages"
        );
        // The stages really came off the sheet: `PilotRuleset::chargen` is `Some`,
        // and its three stages are what the interface was built from.
        let rs = tacsim::sheet::PilotRuleset::new();
        let stages = mush_ruleset::Ruleset::chargen(&rs)
            .expect("the one game sheet has a chargen flow")
            .stages();
        assert_eq!(stages.len(), 3, "{stages:?}");
    }
    use std::sync::{Arc, Mutex};

    use mush_config::config::Config;
    use mush_core::{
        AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey, CanonicalHash,
        CanonicalId, CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind,
        DefinitionRegistryBuilder, GOD, IdempotencyKey, IdempotencyScope, ModError, OwnerId,
        PreparedCausalRef, PrincipalId, Registrar, ReplayWrite, RequestUid, RulesValue,
        ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, World,
    };
    use mush_store::{AuthoritativeStore, InMemoryStore};

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

    #[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct BootState {
        amount: u8,
    }

    struct StateBootMod {
        handle: Arc<Mutex<Option<StateFamilyHandle<u64, BootState>>>>,
        streams_requested: Arc<AtomicBool>,
        loaded: Arc<AtomicBool>,
    }

    impl GameModule for StateBootMod {
        fn name(&self) -> &'static str {
            "state_boot_test"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let descriptor = StateFamilyDescriptor::new(
                StateFamilyId::new("boot").expect("static family id"),
                CanonicalId::new("u64.decimal.v1").expect("static codec id"),
                CanonicalId::new("boot_state.v1").expect("static schema id"),
                1,
                CanonicalId::new("boot_state.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("boot_state.range.v1").expect("static validator id"),
                CanonicalId::new("boot_state.json.v1").expect("static serializer id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?;
            let handle = reg.state_family(
                descriptor,
                |key: &u64| Ok(key.to_string()),
                |value: &serde_json::Value| {
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
                },
                |_value: &BootState| Ok(()),
                |value: &BootState| Ok(serde_json::json!({ "amount": value.amount })),
            )?;
            *self.handle.lock().expect("handle mutex poisoned") = Some(handle);
            Ok(())
        }

        fn boot_stream_requests(
            &self,
            _kv: &[(SmolStr, serde_json::Value)],
        ) -> Result<Vec<(SmolStr, u32)>, ModError> {
            self.streams_requested.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            if data
                .kv
                .iter()
                .any(|(key, _)| is_action_state_persistence_key(key))
            {
                return Err(ModError::msg("reserved state leaked into module boot"));
            }
            self.loaded.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn state_boot_engine() -> (
        Engine,
        StateFamilyHandle<u64, BootState>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    ) {
        let captured = Arc::new(Mutex::new(None));
        let streams_requested = Arc::new(AtomicBool::new(false));
        let loaded = Arc::new(AtomicBool::new(false));
        let engine = Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(StateBootMod {
                handle: Arc::clone(&captured),
                streams_requested: Arc::clone(&streams_requested),
                loaded: Arc::clone(&loaded),
            })],
        )
        .unwrap();
        let handle = captured
            .lock()
            .expect("handle mutex poisoned")
            .clone()
            .expect("registration captured state handle");
        (engine, handle, streams_requested, loaded)
    }

    fn boot_state_op(key: &str, amount: u8) -> ModuleOp {
        ModuleOp {
            ns: "state_boot_test".into(),
            op: ModuleWrite::JsonSet {
                key: format!("action-state/v1/boot/{key}").into(),
                value: serde_json::json!({
                    "family": { "owner": "state_boot_test", "id": "boot" },
                    "key": key,
                    "schema": { "id": "boot_state.v1", "version": 1 },
                    "value": { "amount": amount },
                }),
            },
        }
    }

    /// Seed a module document through the same authoritative prefix owner that
    /// production uses. Reserved `action-state/*` fixtures must never bypass
    /// the store guard through the legacy module-write API.
    async fn seed_authoritative_json(
        store: &InMemoryStore,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
    ) {
        let owner = OwnerId::new(namespace).unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("boot_fixture").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("outcome").unwrap(),
                    DefinitionId::new("boot_fixture_written").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let request = RequestUid::from_bytes([0xA1; 16]);
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([0xA2; 16]),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: PrincipalId::Object { id: GOD },
                    owner: owner.clone(),
                    namespace: CanonicalId::new("boot_fixture").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes([0xA3; 32]),
            request_hash: CanonicalHash::from_bytes([0xA4; 32]),
            plan_hash: CanonicalHash::from_bytes([0xA5; 32]),
            action: action.clone(),
            provider: owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action.clone(), outcome.clone()],
            writes: vec![ReplayWrite::ModuleJsonSet {
                namespace: owner,
                key: AuthoritativeKey::parse_canonical(key).unwrap(),
                value,
            }],
            outcome: RulesValue::new(outcome.clone(), serde_json::json!({"seeded": true})).unwrap(),
            resolution: None,
            semantic_events: vec![
                mush_core::AuthoritativeSemanticEvent::root(
                    PreparedCausalRef::new(request, None, 0).unwrap(),
                    &action,
                    RulesValue::new(outcome, serde_json::json!({"seeded": true})).unwrap(),
                    vec![PrincipalId::Object { id: GOD }],
                )
                .unwrap(),
            ],
            eligible_witnesses: vec![PrincipalId::Object { id: GOD }],
            durable_outbox: vec![],
        })
        .unwrap();
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        store.reserve_authoritative_intent(bundle).await.unwrap();
        store.reconcile_authoritative(token, hash).await.unwrap();
        store.checkpoint_authoritative(token, hash).await.unwrap();
    }

    async fn seed_authoritative_op(store: &InMemoryStore, op: ModuleOp) {
        let ModuleWrite::JsonSet { key, value } = op.op else {
            panic!("authoritative boot fixture must be a JSON set");
        };
        seed_authoritative_json(store, &op.ns, &key, value).await;
    }

    struct ReservedBootWrite;

    impl GameModule for ReservedBootWrite {
        fn name(&self) -> &'static str {
            "reserved_boot_write"
        }

        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            ctx: &mut mush_core::ModuleCtx<'_>,
            _data: ModuleBootData,
        ) -> Result<(), ModError> {
            ctx.persist(ModuleOp {
                ns: self.name().into(),
                op: ModuleWrite::JsonSet {
                    key: "action-state/v1/forged/1".into(),
                    value: serde_json::json!(true),
                },
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn module_boot_cannot_write_the_reserved_action_state_keyspace() {
        let store = InMemoryStore::new();
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(ReservedBootWrite)],
        )
        .unwrap();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("coordinator-reserved"), "{error}");
        assert!(
            store
                .module_json_get("reserved_boot_write", "action-state/v1/forged/1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn boot_hydrates_typed_state_before_module_callbacks() {
        let store = InMemoryStore::new();
        seed_authoritative_op(&store, boot_state_op("7", 3)).await;
        let (mut engine, handle, streams_requested, loaded) = state_boot_engine();

        boot_modules(&mut engine, &store).await.unwrap();

        assert!(streams_requested.load(Ordering::SeqCst));
        assert!(loaded.load(Ordering::SeqCst));
        assert_eq!(
            engine.action_state().get(&handle, &7).unwrap(),
            Some(&BootState { amount: 3 })
        );
    }

    #[tokio::test]
    async fn malformed_typed_state_aborts_before_any_module_callback() {
        let store = InMemoryStore::new();
        let mut op = boot_state_op("7", 3);
        let ModuleWrite::JsonSet { value, .. } = &mut op.op else {
            unreachable!()
        };
        value["schema"]["version"] = serde_json::json!(2);
        seed_authoritative_op(&store, op).await;
        let (mut engine, handle, streams_requested, loaded) = state_boot_engine();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("schema.version"), "{error}");
        assert!(!streams_requested.load(Ordering::SeqCst));
        assert!(!loaded.load(Ordering::SeqCst));
        assert!(engine.action_state().get(&handle, &7).unwrap().is_none());
    }

    #[tokio::test]
    async fn future_action_state_encoding_is_reserved_and_fails_closed() {
        let store = InMemoryStore::new();
        let mut op = boot_state_op("7", 3);
        let ModuleWrite::JsonSet { key, .. } = &mut op.op else {
            unreachable!()
        };
        *key = "action-state/v2/boot/7".into();
        seed_authoritative_op(&store, op).await;
        let (mut engine, _, streams_requested, loaded) = state_boot_engine();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("unsupported action-state"), "{error}");
        assert!(!streams_requested.load(Ordering::SeqCst));
        assert!(!loaded.load(Ordering::SeqCst));
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
    async fn module_migrations_never_transform_reserved_action_state() {
        let store = InMemoryStore::new();
        let reserved = serde_json::json!({ "a": "must remain byte-equivalent" });
        seed_authoritative_json(&store, "toy", "action-state/v1/example/1", reserved.clone()).await;
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
        let engine = toy_engine();

        run_module_migrations(&store, engine.registry.migrations())
            .await
            .unwrap();

        assert_eq!(
            store
                .module_json_get("toy", "action-state/v1/example/1")
                .await
                .unwrap(),
            Some(reserved)
        );
        assert_eq!(
            store.module_json_get("toy", "doc:1").await.unwrap(),
            Some(serde_json::json!({ "b": 42 }))
        );
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
        let boot = load_ordinary_module_boot(&store, &modules).await.unwrap();
        let kv = &boot[&SmolStr::new("toy")].kv;
        assert_eq!(kv.len(), 1);
        assert_eq!(kv[0].0, "doc:1");
    }

    /// J4 (doc 12 §5.6): the loader reads the namespace's boot manifest first
    /// and fetches nothing it names; the omission reaches the module as the
    /// min-present marker; the manifest itself never does; a whole fetch
    /// ignores it.
    #[tokio::test]
    async fn j4_the_loader_narrows_the_fetch_to_the_boot_manifest() {
        let store = InMemoryStore::new();
        let chunk = |n: u64, sealed: bool| {
            serde_json::json!({
                "chunk": n, "sealed": sealed, "entries": [{ "seq": n, "value": { "n": n } }]
            })
        };
        let set = |key: &str, value: serde_json::Value| ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::JsonSet {
                key: key.into(),
                value,
            },
        };
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                set("doc:1", serde_json::json!({ "b": 1 })),
                set("log:s:0", chunk(0, true)),
                set("log:s:1", chunk(1, true)),
                set("log:s:2", chunk(2, false)),
                set("log:t:0", chunk(0, false)),
                set("idem:aa", serde_json::json!({ "tx": 1 })),
                set(
                    BOOT_MANIFEST_KEY,
                    serde_json::to_value(BootManifest {
                        format: mush_core::BOOT_MANIFEST_FORMAT,
                        skip_log_chunks_below: [(SmolStr::new("s"), 2u64)].into_iter().collect(),
                        skip_key_prefixes: vec![SmolStr::new("idem:")],
                    })
                    .unwrap(),
                ),
            ],
        )
        .await
        .unwrap();
        let (data, _) = load_one_namespace(&store, "toy", Fetch::Narrowed)
            .await
            .unwrap();
        assert_eq!(
            data.kv.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["doc:1"],
            "the manifest is loader-owned and never a module document; the prefixed key is not fetched"
        );
        let mut ledger: Vec<&str> = data.ledger.iter().map(|(k, _)| k.as_str()).collect();
        ledger.sort_unstable();
        assert_eq!(ledger, vec!["log:s:2", "log:t:0"]);
        assert_eq!(
            data.omitted,
            [(SmolStr::new("s"), 2u64)].into_iter().collect()
        );
        let (whole, _) = load_one_namespace(&store, "toy", Fetch::Whole)
            .await
            .unwrap();
        assert_eq!(
            whole.ledger.len(),
            4,
            "a whole fetch ignores the manifest's floors"
        );
        assert!(whole.omitted.is_empty());
        assert_eq!(
            whole.kv.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["doc:1"],
            "a whole fetch still honours the manifest's key prefixes"
        );
        // An unreadable manifest is a whole fetch, not a failed boot.
        write_ops(
            &store,
            vec![set(
                BOOT_MANIFEST_KEY,
                serde_json::json!({ "format": "nine" }),
            )],
        )
        .await
        .unwrap();
        let (data, _) = load_one_namespace(&store, "toy", Fetch::Narrowed)
            .await
            .unwrap();
        assert_eq!(data.ledger.len(), 4);
        assert!(data.omitted.is_empty());
        // A manifest at a format this loader does not know: the same.
        write_ops(
            &store,
            vec![set(
                BOOT_MANIFEST_KEY,
                serde_json::json!({ "format": 99, "skip_log_chunks_below": { "s": 2 } }),
            )],
        )
        .await
        .unwrap();
        let (data, _) = load_one_namespace(&store, "toy", Fetch::Narrowed)
            .await
            .unwrap();
        assert_eq!(data.ledger.len(), 4);
    }

    /// A module that refuses a narrowed namespace once (J4's
    /// `RetryFullFetch`) and records what each delivery carried.
    struct PickyMod {
        deliveries: Arc<Mutex<Vec<(usize, usize)>>>,
        refuse_narrowed: bool,
    }

    impl GameModule for PickyMod {
        fn name(&self) -> &'static str {
            "picky"
        }

        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            self.deliveries
                .lock()
                .unwrap()
                .push((data.ledger.len(), data.omitted.len()));
            if self.refuse_narrowed && !data.omitted.is_empty() {
                return Err(ModError::RetryFullFetch {
                    module: SmolStr::new("picky"),
                    reason: "the checkpoint the manifest stood for is gone".into(),
                });
            }
            Ok(())
        }
    }

    /// J4 (lens G): a module that cannot boot from its narrowed namespace is
    /// delivered the whole one, itself alone, and the boot goes on; a module
    /// that refuses the whole one too fails the boot; the retry is not
    /// offered to a whole fetch.
    #[tokio::test]
    async fn j4_boot_modules_refetches_whole_for_a_module_that_asks() {
        let store = InMemoryStore::new();
        let chunk = |n: u64, sealed: bool| {
            serde_json::json!({
                "chunk": n, "sealed": sealed, "entries": [{ "seq": n, "value": { "n": n } }]
            })
        };
        let set = |key: &str, value: serde_json::Value| ModuleOp {
            ns: "picky".into(),
            op: ModuleWrite::JsonSet {
                key: key.into(),
                value,
            },
        };
        write_ops(
            &store,
            vec![
                set("log:s:0", chunk(0, true)),
                set("log:s:1", chunk(1, false)),
                set(
                    BOOT_MANIFEST_KEY,
                    serde_json::to_value(BootManifest {
                        format: mush_core::BOOT_MANIFEST_FORMAT,
                        skip_log_chunks_below: [(SmolStr::new("s"), 1u64)].into_iter().collect(),
                        skip_key_prefixes: vec![SmolStr::new("idem:")],
                    })
                    .unwrap(),
                ),
            ],
        )
        .await
        .unwrap();
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        let picky = PickyMod {
            deliveries: Arc::clone(&deliveries),
            refuse_narrowed: true,
        };
        let modules: Vec<Box<dyn GameModule>> =
            vec![Box::new(picky), Box::new(ToyMod { loaded: Vec::new() })];
        let mut engine =
            Engine::with_modules(World::new_minimal(), mush_core::Registry::new(), modules)
                .unwrap();
        boot_modules(&mut engine, &store).await.unwrap();
        assert_eq!(
            *deliveries.lock().unwrap(),
            vec![(1, 1), (2, 0)],
            "narrowed first (one chunk, one floor), then whole (two chunks, no floor)"
        );
        assert_eq!(
            store
                .module_json_get("picky", BOOT_MANIFEST_KEY)
                .await
                .unwrap()
                .map(|doc| serde_json::from_value::<BootManifest>(doc).unwrap()),
            Some(BootManifest {
                format: mush_core::BOOT_MANIFEST_FORMAT,
                skip_log_chunks_below: std::collections::BTreeMap::new(),
                skip_key_prefixes: vec![SmolStr::new("idem:")],
            }),
            "the floors that stood for nothing are cleared by the loader; the key prefixes are kept"
        );

        // The whole fetch refused too: the boot fails with the module's reason.
        // (The first retry cleared the manifest; write it back for this one.)
        write_ops(
            &store,
            vec![set(
                BOOT_MANIFEST_KEY,
                serde_json::to_value(BootManifest {
                    format: mush_core::BOOT_MANIFEST_FORMAT,
                    skip_log_chunks_below: [(SmolStr::new("s"), 1u64)].into_iter().collect(),
                    skip_key_prefixes: Vec::new(),
                })
                .unwrap(),
            )],
        )
        .await
        .unwrap();
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        struct AlwaysRetry(Arc<Mutex<Vec<(usize, usize)>>>);
        impl GameModule for AlwaysRetry {
            fn name(&self) -> &'static str {
                "picky"
            }
            fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
                Ok(())
            }
            fn on_world_loaded(
                &mut self,
                _ctx: &mut mush_core::ModuleCtx<'_>,
                data: ModuleBootData,
            ) -> Result<(), ModError> {
                self.0
                    .lock()
                    .unwrap()
                    .push((data.ledger.len(), data.omitted.len()));
                Err(ModError::RetryFullFetch {
                    module: SmolStr::new("picky"),
                    reason: "never satisfied".into(),
                })
            }
        }
        let modules: Vec<Box<dyn GameModule>> =
            vec![Box::new(AlwaysRetry(Arc::clone(&deliveries)))];
        let mut engine =
            Engine::with_modules(World::new_minimal(), mush_core::Registry::new(), modules)
                .unwrap();
        let err = boot_modules(&mut engine, &store).await.unwrap_err();
        assert!(err.contains("after the whole fetch"), "{err}");
        assert!(err.contains("never satisfied"), "{err}");
        assert_eq!(
            *deliveries.lock().unwrap(),
            vec![(1, 1), (2, 0)],
            "exactly one retry"
        );
    }

    /// J1: ledger chunks reach boot data as text, apart from the ordinary
    /// documents, and the namespace fetch is timed.
    #[tokio::test]
    async fn j1_ledger_chunks_reach_boot_data_as_text() {
        let store = InMemoryStore::new();
        let chunk = serde_json::json!({
            "chunk": 0, "sealed": false, "entries": [{ "seq": 0, "value": { "n": 1 } }]
        });
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
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "log:s:0".into(),
                        value: chunk.clone(),
                    },
                },
            ],
        )
        .await
        .unwrap();
        let modules: Vec<Box<dyn GameModule>> = vec![Box::new(ToyMod { loaded: Vec::new() })];
        let boot = load_ordinary_module_boot(&store, &modules).await.unwrap();
        let data = &boot[&SmolStr::new("toy")];
        assert_eq!(data.kv.len(), 1);
        assert_eq!(data.kv[0].0, "doc:1");
        assert_eq!(data.ledger.len(), 1);
        assert_eq!(data.ledger[0].0, "log:s:0");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&data.ledger[0].1).unwrap(),
            chunk
        );
        assert!(data.fetch_ms < 60_000, "a timed fetch: {}", data.fetch_ms);
    }

    #[tokio::test]
    async fn action_state_keys_never_reach_module_boot_data() {
        let store = InMemoryStore::new();
        seed_authoritative_json(
            &store,
            "toy",
            "action-state/v99/future/1",
            serde_json::json!({ "opaque": true }),
        )
        .await;
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

        let boot = load_ordinary_module_boot(&store, &modules).await.unwrap();

        assert_eq!(boot[&SmolStr::new("toy")].kv.len(), 1);
        assert_eq!(boot[&SmolStr::new("toy")].kv[0].0, "doc:1");
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

    // ---- WS04-S8: the CI-gated composition IS the production composition ----
    //
    // `.github/workflows/ci.yml`'s K8 kernel-contract job runs
    // `k8_5_production_compositions_publish_command_compatibility` by name, and
    // that test used to hand-assemble the roster it certified:
    // `server_modules_with` + `sheet_modules` + one
    // `tacsim_module(tacsim::fixture::config())`. So the one job whose
    // name promises production fidelity asserted over a list nobody boots — no
    // worldgen, no NPC agent, `RecoveryConfig::default()` where a server passes
    // the operator's `[tacsim.recovery]`, and a compiled-in fixture where a
    // server reads pack FILES off disk. That is how the split half's missing
    // TacSim layer stayed green through review until WS04-S2 pushed it.
    //
    // The roster below now comes from `boot::game_modules` — the one root both
    // halves call — so the gate reads what a server composes. It is the claim
    // `tests/boot_parity.rs` makes from the integration side; this is the
    // `--lib` copy, because a `--lib` test is what CI names.

    /// A memory-backed config on a scratch data dir, with the corridor fixture
    /// written out as the two pack FILES a host configures — the path an operator's
    /// own pack takes, digests and all (`tests/boot_parity.rs`'s `enable_tacsim`
    /// shape).
    ///
    /// 16b cut R-b: no `[rulesets].enabled` id is set, because nothing reads one —
    /// there is exactly one composition.
    fn production_config(tag: &str) -> Config {
        let dir = std::env::temp_dir().join(format!(
            "mush-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let packs = dir.join("packs");
        std::fs::create_dir_all(&packs).expect("a scratch pack dir");
        let core = packs.join("tacsim_core.json");
        let corridor = packs.join("tacsim_corridor.json");
        std::fs::write(&core, tacsim::fixture::CORE_JSON).expect("the core pack file");
        std::fs::write(&corridor, tacsim::fixture::CORRIDOR_JSON).expect("the corridor seed file");
        let mut cfg = Config::default();
        cfg.store.store = "memory".to_string();
        cfg.server.data_dir = dir;
        cfg.tacsim.enabled = true;
        cfg.tacsim.pack_paths = vec![core.display().to_string()];
        cfg.tacsim.seed_paths = vec![corridor.display().to_string()];
        cfg.tacsim.corridor = tacsim::fixture::CORRIDOR_PACK.to_string();
        cfg
    }

    /// The production roster, composed through [`crate::boot::game_modules`],
    /// beside the scratch dir its packs were read from — removed on drop, so a
    /// failing case leaves nothing behind.
    struct ProductionRoster {
        cfg: Config,
        modules: Vec<Box<dyn GameModule>>,
    }

    impl ProductionRoster {
        /// `game_modules` is sync, so no runtime is needed; it composes worldgen
        /// and the NPC agent, so it needs an `LlmService` — keyless, which still
        /// builds and speaks to nothing.
        fn build(tag: &str) -> Self {
            let cfg = production_config(tag);
            let llm_config = crate::config_ext::llm_config(&cfg.llm).expect("[llm] parses");
            let llm = Arc::new(
                LlmService::from_config(&llm_config).expect("a keyless service still builds"),
            );
            let modules =
                crate::boot::game_modules(&cfg, &llm).expect("the production roster composes");
            Self { cfg, modules }
        }

        fn names(&self) -> Vec<&str> {
            self.modules.iter().map(|m| m.name()).collect()
        }

        /// The composed list, moved out for `Engine::compose_modules`.
        fn take(&mut self) -> Vec<Box<dyn GameModule>> {
            std::mem::take(&mut self.modules)
        }
    }

    impl Drop for ProductionRoster {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.cfg.server.data_dir);
        }
    }

    /// The roster the CI gate used to hand-assemble, kept ONLY as an explicitly
    /// documented negative control: it composes, and it is not what a server
    /// boots — which is exactly why certifying it proved nothing.
    fn hand_built_roster() -> Vec<Box<dyn GameModule>> {
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let mut modules = server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            mush_campaign::RecoveryConfig::default(),
        );
        modules.extend(sheet_modules(&onboarding, &game));
        modules.push(tacsim_module(tacsim::fixture::config()));
        modules
    }

    /// WS04-S8: the roster the CI gate composes, as an ordered golden, beside
    /// the hand-built list it replaced, as the negative control.
    ///
    /// Drift becomes a named failure rather than a silent re-certification: a
    /// module added, dropped or reordered in `boot::game_modules` fails this
    /// assert with `boot.rs` in the message. (`tests/boot_parity.rs`'s
    /// `expected_roster` states the same golden from the integration side; the
    /// two must agree, and this is the copy the `--lib` gate can see.)
    ///
    /// 16b cut R-b: there is ONE composition, so the matrix this used to loop
    /// over is gone with the two character rulesets it selected. The property is
    /// unchanged — the certified roster is the roster a server boots.
    #[test]
    fn the_ci_gated_composition_is_the_boot_composition() {
        let roster = ProductionRoster::build("k85-order");
        let expected = [
            "comsys",
            "canon",
            "privileges",
            "org",
            "faction",
            "scenes",
            "sim",
            // S1: the atlas, right after the placement graph S2's journeys read.
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
            // Roll opens readiness before Market's final renewal recovery audit.
            "roll",
            "market",
            "script",
            // The composition root's own two, after the built-ins — the pair
            // the hand-built roster never had.
            "worldgen",
            "npc",
            // ...then the one game sheet, and (S6) the rules interface composed
            // beside it...
            "pilot",
            "rules_interface",
            // ...the TacSim layer next: its boot reads that sheet's facts...
            "tacsim",
            // ...and the content module LAST: its boot pass drives every other
            // module's appliers, so each owner hydrates before it runs (S1).
            "content",
        ];
        assert_eq!(
            roster.names(),
            expected,
            "the roster drifted from the golden; if that was deliberate, \
             `boot::game_modules` and this list move together"
        );

        let hand_built: Vec<&str> = hand_built_roster().iter().map(|m| m.name()).collect();
        assert_ne!(
            hand_built,
            roster.names(),
            "the hand-built roster is kept only as a negative control; if it has \
             become the production roster, delete it and this half of the test"
        );
        for missing in ["worldgen", "npc"] {
            assert!(
                !hand_built.contains(&missing) && roster.names().contains(&missing),
                "{missing} is what the hand-built roster leaves out"
            );
        }
    }

    #[test]
    fn k8_5_production_compositions_publish_command_compatibility() {
        {
            // WS04-S8: the roster a server boots, read through the one root, with
            // the TacSim layer composed from pack FILES. So this gate's `@tacsim`
            // row fails the day `boot::game_modules` stops pushing the layer —
            // the property the hand-built list could never have.
            //
            // 16b cut R-b: one composition, so the `{wod, shadowrun}` loop this
            // block used to sit inside is gone. Every row below is unchanged.
            let mut roster = ProductionRoster::build("k85");
            let composition =
                Engine::compose_modules(crate::world_task::base_registry(), roster.take())
                    .expect("production composition declares every command lane");

            let registrations = composition
                .registry()
                .command_registrations()
                .collect::<Vec<_>>();
            assert!(
                registrations.len() > 50,
                "unexpectedly small command surface"
            );
            assert!(
                registrations.iter().any(|(spec, owner, class)| {
                    spec.name == "@tacsim"
                        && *owner == Some("tacsim")
                        && *class == mush_core::CommandCompatibility::LegacyPostImage
                }),
                "the TacSim layer must publish its compatibility lane"
            );
            // E7 residue pass 1, R2: `@corridor` is a SECOND registration group in the
            // same module, declared `ReadOnly` before its own `reg.command` and with
            // `LegacyPostImage` restored immediately after — the K8.5 discipline that
            // failed `mush-market`'s 140 tests at the K merge, applied per group.
            assert!(
                registrations.iter().any(|(spec, owner, class)| {
                    spec.name == "@corridor"
                        && *owner == Some("tacsim")
                        && *class == mush_core::CommandCompatibility::ReadOnly
                }),
                "the corridor read surface must publish `ReadOnly`"
            );
            // E7 cut 2: the FIRST `Authoritative` command registration on the
            // tree. `@inv` is one `CommandSpec`, so it is one registration and
            // takes the ceiling class of its nine switches.
            assert!(
                registrations.iter().any(|(spec, owner, class)| {
                    spec.name == "@inv"
                        && *owner == Some("inventory")
                        && *class == mush_core::CommandCompatibility::Authoritative
                }),
                "the inventory lane must publish `Authoritative`"
            );
            // E7 cut 3: five lanes in THREE registration groups, each with its
            // own `reg.classify_commands(..)` before its first `reg.command`.
            // `@deposit` is `Authoritative` rather than `ReadOnly` because a
            // body's reserve IS cut 2's pool and a pool can only be credited
            // inside an authoritative bundle — conservative over-declaration,
            // and the two read switches simply never submit.
            for (name, class) in [
                ("@deposit", mush_core::CommandCompatibility::Authoritative),
                ("@claim", mush_core::CommandCompatibility::Authoritative),
                ("@extract", mush_core::CommandCompatibility::Authoritative),
                ("@survey", mush_core::CommandCompatibility::LegacyPostImage),
                ("@licence", mush_core::CommandCompatibility::ReadOnly),
                // E7 cut 4: the work-order lane (ceiling Authoritative) and the
                // facility lane (LegacyPostImage — open writes a document).
                ("@wo", mush_core::CommandCompatibility::Authoritative),
                (
                    "@facility",
                    mush_core::CommandCompatibility::LegacyPostImage,
                ),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("industry") && *published == class
                    }),
                    "the industry lane {name} must publish {class:?}"
                );
            }
            // E7 cut 5: `@station` is one CommandSpec, so it takes the CEILING
            // class of its switches — `/handoff` and `/cancel` ride cut 2's
            // authoritative control-lease ingress, so the sim lane publishes
            // `Authoritative` (`/show` reads and `/fallback` writes the exempt
            // `sim.station_fallback` record under the same lane).
            //
            // E7 cut 6 (fold B10): `@journey` is the sim lane's other E7 verb
            // and had no row here — it is covered by the allowlist tripwire and
            // by `e7c6_p4`, but `e7c6_p4` builds a two-module `TestWorld`, so
            // nothing caught a composition-order change that re-classified it.
            // A ROW in this loop, not a new stanza: `PINNED_BASELINE` is
            // untouched.
            //
            // 16b cut T (T0, D-16-20): the eight tactical verbs
            // (`+fire +launch +heading +thrust +posture +dc +dock +eject`) and
            // the `tactical_intent` action they staged are RETIRED — combat is
            // the arena's. Their rows leave this loop with their stanzas;
            // `PINNED_BASELINE` does not move, because all eight were
            // `authoritative` and never baseline.
            //
            // E7 cut 8 independent verify fold (B1): `@domain` publishes
            // `Authoritative`. Its cut-6 note here — "the durable cursor is
            // armed by the `sim.domain_pulse` Coarse tick, never by the
            // command" — was factually false and is deleted: `@domain/tick`
            // runs `tick::run_maintenance`, which reaches
            // `Ctx::submit_authoritative(domain_pulse_arm, ..)` from the
            // command handler, and cut 8's own `e7c8_x3_*` proved it by
            // draining that submission. Its allowlist stanza moves
            // `baseline` → `authoritative` in the same commit, which lowers
            // `PINNED_BASELINE` 158 → 157 — the retirement the tripwire itself
            // documents as legal.
            for (name, class) in [
                ("@station", mush_core::CommandCompatibility::Authoritative),
                ("@journey", mush_core::CommandCompatibility::LegacyPostImage),
                ("@domain", mush_core::CommandCompatibility::Authoritative),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("sim") && *published == class
                    }),
                    "the sim lane {name} must publish {class:?}"
                );
            }
            // E7 cut 7: FIVE verbs in THREE registration groups, each with its
            // own `reg.classify_commands(..)` before its first `reg.command`.
            // `+contacts`/`+contact` are pure reads over the leak-safe ledger;
            // `+order` writes propagation's own exempt documents, and `+orders`
            // takes the CEILING class of its switches because `/ack` writes the
            // acknowledgement envelope. `@prop` takes the ceiling of ITS
            // switches too, and `/pass` stages the tactics-owned `sensor_arm`
            // through `Ctx::submit_authoritative` — an ingress adapter for the
            // authoritative action coordinator, which is `Authoritative`'s own
            // definition and exactly the criterion cut 5 used for `@station`
            // (E7 cut 7 independent verify fold, B8).
            for (name, class) in [
                ("+contacts", mush_core::CommandCompatibility::ReadOnly),
                ("+contact", mush_core::CommandCompatibility::ReadOnly),
                ("+order", mush_core::CommandCompatibility::LegacyPostImage),
                ("+orders", mush_core::CommandCompatibility::LegacyPostImage),
                ("@prop", mush_core::CommandCompatibility::Authoritative),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("propagation") && *published == class
                    }),
                    "the propagation lane {name} must publish {class:?}"
                );
            }
            // E7 cut 9: THREE verbs in TWO registration groups, each with its own
            // `reg.classify_commands(..)` before its first `reg.command`, and
            // `LegacyPostImage` restored last so the module ends in the class its
            // own documents ride. A `CommandSpec` is one PRIMARY VERB, so each
            // takes the CEILING class of its switches — the rule the tree already
            // states for `@station`, `@journey`, `@prop` and `@domain`.
            // `@formation` reads only. `@op` and `@force` write the exempt
            // `campaign.operation` / `campaign.formation` documents, and
            // `@op/activate` additionally posts an E4 `HoldOp::Place` through
            // `mush_economy::post` — the economy's OWN carve-out lane, never
            // `Ctx::submit_authoritative` — so the ceiling is `LegacyPostImage`
            // and NOT `Authoritative`.
            for (name, class) in [
                ("@formation", mush_core::CommandCompatibility::ReadOnly),
                ("@op", mush_core::CommandCompatibility::LegacyPostImage),
                ("@force", mush_core::CommandCompatibility::LegacyPostImage),
                // E7 residue pass 1, R3: `@recovery` is its own registration group,
                // classified before its own `reg.command` and restored after. Its
                // ceiling is `LegacyPostImage` for `@op`'s exact reason — the three
                // `recovery.queue` documents are exempt workflow records and the
                // outfitting stipend rides `mush_economy::post`'s carve-out lane
                // from the command, never `Ctx::submit_authoritative`.
                (
                    "@recovery",
                    mush_core::CommandCompatibility::LegacyPostImage,
                ),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("campaign") && *published == class
                    }),
                    "the campaign lane {name} must publish {class:?}"
                );
            }
            // ---- E7r2: P1 — the market lane moves to `Authoritative` ----
            //
            // Both verbs, by the CEILING rule a `CommandSpec` takes over its switches
            // (`@station` / `@inv` / `@wo` / `@prop` / `@domain`'s precedent):
            // `@market/vendor stock`, `@market/vendor withdraw` and `+market/accept`
            // each stage exactly one `Ctx::submit_authoritative` of the market-owned
            // `market_lot` action, which reserves, settles and releases on cut 2's
            // delegated `InventoryFamilies`. `crate::commands::register` classifies the
            // group before its first `reg.command` and restores `LegacyPostImage`
            // after, so the module still ends in the class its own `market:*` documents
            // ride. Both allowlist categories move in the same commit and
            // `PINNED_BASELINE` is unmoved at 157 — neither stanza was baseline.
            for (name, class) in [
                ("+market", mush_core::CommandCompatibility::Authoritative),
                ("@market", mush_core::CommandCompatibility::Authoritative),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("market") && *published == class
                    }),
                    "the market lane {name} must publish {class:?}"
                );
            }
            // E7 cut 10: the seat lane, in TWO further groups on the same
            // module. `+seat` and `@seat` publish `Authoritative` because
            // `/activate`, `/leave` and `/revoke` reach K6's control lease
            // through cut 5's ingress (`@station`'s exact precedent — the
            // ingress publishes the class, whoever stages it); `@crew`,
            // `+crewcall` and `+muster` write only the exempt `campaign.crew` /
            // `campaign.muster` workflow documents, and `LegacyPostImage` is
            // restored last so the module still ends in the class its own
            // records ride.
            for (name, class) in [
                ("+seat", mush_core::CommandCompatibility::Authoritative),
                ("@seat", mush_core::CommandCompatibility::Authoritative),
                (
                    "+crewcall",
                    mush_core::CommandCompatibility::LegacyPostImage,
                ),
                ("+muster", mush_core::CommandCompatibility::LegacyPostImage),
                ("@crew", mush_core::CommandCompatibility::LegacyPostImage),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("campaign") && *published == class
                    }),
                    "the campaign seat lane {name} must publish {class:?}"
                );
            }
            // ---- 16b cut R-b: the one game sheet's own lane ----
            //
            // E7r2 P0c's `+nuyen`/`@sr` rows left with `ruleset-shadowrun`. What
            // replaces them is the same claim about the module that replaced it:
            // `PilotRuleset::register` classifies its whole group
            // `LegacyPostImage` before its first `reg.command` (a sheet write is a
            // bounded whole-document post-image), so every verb it publishes rides
            // that one lane and none of them silently rides another.
            for name in ["+sheet", "@sheet", "+chargen", "+stat", "+roll"] {
                assert!(
                    registrations.iter().any(|(spec, owner, class)| {
                        spec.name == name
                            && *owner == Some("pilot")
                            && *class == mush_core::CommandCompatibility::LegacyPostImage
                    }),
                    "the pilot sheet lane {name} must publish `LegacyPostImage`"
                );
            }
            assert!(registrations.iter().any(|(_, owner, class)| {
                *owner == Some("economy")
                    && *class == mush_core::CommandCompatibility::EconomyCarveOut
            }));
            assert!(registrations.iter().all(|(_, owner, class)| {
                *owner != Some("economy")
                    || *class == mush_core::CommandCompatibility::EconomyCarveOut
            }));
            assert!(
                composition
                    .registry()
                    .command_fallback_compatibilities()
                    .any(|(owner, class)| {
                        owner == Some("script")
                            && class == mush_core::CommandCompatibility::LegacyPostImage
                    })
            );
        }
    }

    /// fu3-4: the new composition-freeze refusal — no global verb may capture a
    /// verb some registered behavior declares — must not wedge a real boot.
    ///
    /// The refusal is a hard error, so the roster it runs over has to be
    /// checked, not assumed. Today no shipped `BehaviorSpec` declares
    /// contextual commands at all, so the pass has nothing to refuse; this test
    /// is what says so out loud and what fails the day a module adds a behavior
    /// command whose verb the global table already answers.
    /// (The unit proof of the mechanism itself lives in `mush-core`'s
    /// `tests/module_api.rs`; it cannot build a production roster from there
    /// without `mush-core` depending on `mush-server`.)
    #[test]
    fn production_composition_registers_no_shadowing_verb() {
        // WS04-S8: the same production root `k8_5` reads, for the same
        // reason — a refusal checked over a roster nobody boots says nothing
        // about the boot.
        let mut roster = ProductionRoster::build("shadowing");
        Engine::compose_modules(crate::world_task::base_registry(), roster.take()).unwrap_or_else(
            |error| panic!("the composition must not shadow a behavior verb: {error}"),
        );
    }

    /// E7 cut 3: the claim's titled object is minted by
    /// `mush_economy::issue_title` under `industry.claims`, which `issue_title`
    /// refuses unless it is on the economy's own issuer list. The two names
    /// have to agree, so a typo is a loud refusal at the verb rather than a
    /// silent capability — this pins that they do.
    #[test]
    fn e7c3_wiring_the_industry_title_issuer_is_registered_with_the_economy() {
        use mush_testkit::TestWorld;
        let onboarding = mush_core::OnboardingConfig::default();
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &tacsim::GameEconomyConfig::default(),
            mush_campaign::RecoveryConfig::default(),
        ));
        // The economy composed at all, and the industry store came with it —
        // which is the composition this test is really about.
        assert!(
            tw.world()
                .module_state::<mush_industry::IndustryStore>()
                .is_some(),
            "mush-industry composes on every ruleset"
        );
        assert_eq!(
            mush_industry::industry_title_issuer().as_str(),
            "industry.claims"
        );
        // And the composition root passed exactly that principal to
        // `with_title_issuers`: a `@claim/file` that reached `issue_title` with
        // an unregistered issuer would answer `Unauthorized`, so the pin is the
        // source line itself.
        let source = include_str!("modules.rs");
        assert!(
            source.contains("issuers.push(mush_industry::industry_title_issuer());"),
            "the industry issuer must be on the economy's list"
        );
    }

    /// E7 cut 3 B1: the cross-owner write bridge. `mush-inventory` delegates
    /// its typed families to `"industry"` at composition, which is what lets one
    /// extraction bundle carry both owners' post-images. It is never implicit:
    /// no other owner is a grantee, and the delegation is not symmetric.
    ///
    /// **E7r2 P1 adds the second grantee and nothing else.** `"market"` joins on
    /// exactly the same terms — a market-owned action's prepare drives the same
    /// exported verbs under `mush_market::listing_holder` — and the two properties this
    /// test exists for are unchanged: the delegation is still **one-directional** (a
    /// delegate cannot write back), and a roster without the TacSim layer still carries
    /// **only** the delegations named here, so a third one cannot arrive unnoticed.
    ///
    /// **Doc 18 cut IC5** (D-18-33): the third one arrived, noticed — but only on a
    /// roster that composes the TacSim layer (`server_modules_with_tacsim`). This
    /// roster is `server_modules_with` alone, which still carries exactly the two;
    /// the game roster's three are pinned by
    /// [`ic5_wiring_tacsim_is_the_third_quantity_delegate_on_the_game_roster`].
    #[test]
    fn e7c3_wiring_the_quantity_delegation_is_declared_and_narrow() {
        use mush_testkit::TestWorld;
        let onboarding = mush_core::OnboardingConfig::default();
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &tacsim::GameEconomyConfig::default(),
            mush_campaign::RecoveryConfig::default(),
        ));
        let families = tw
            .engine()
            .registry
            .state_families()
            .expect("the composed state-family registry");
        let inventory = mush_core::OwnerId::new("inventory").expect("owner");
        let industry = mush_core::OwnerId::new("industry").expect("owner");
        let market = mush_core::OwnerId::new("market").expect("owner");
        assert!(
            families.delegates(&inventory, &industry),
            "inventory delegates its families to industry"
        );
        assert!(
            families.delegates(&inventory, &market),
            "E7r2 P1: …and to market, which is what lets one sale carry the pool, the \
             reservation and the buyer's lot in one bundle"
        );
        assert!(
            !families.delegates(&industry, &inventory),
            "and the delegation is NOT symmetric"
        );
        assert!(
            !families.delegates(&market, &inventory),
            "…in either direction: the market grants the quantity authority nothing"
        );
        assert!(
            !families.delegates(&market, &industry) && !families.delegates(&industry, &market),
            "and the two grantees are strangers to each other"
        );
        assert_eq!(
            families.delegations(),
            vec![(inventory.clone(), industry), (inventory, market)],
            "a roster without the TacSim layer carries exactly the two delegations, both \
             from `inventory` outward"
        );
    }

    /// **Doc 18 cut IC5 — the composition-edge pin** (D-18-33; §4.4 "Offline
    /// execution" prerequisite). The production roster both binaries boot
    /// (`crate::boot::game_modules`) composes the TacSim layer, so `mush-inventory`
    /// names `tacsim` its THIRD quantity delegate there — and nowhere else: the
    /// delegation travels with the layer (`server_modules_with_tacsim`). It is
    /// one-directional like the other two.
    ///
    /// **Doc 20 cut SM.6 (D1) moves the pin from three pairs to five**: on this roster
    /// only, `mush-industry` delegates its typed families to `tacsim` (the port tick and
    /// readiness read deposits) and `tacsim` delegates its own to `industry` (the
    /// sweep's `ExternalDraws` reads `tacsim.pockets`) — each widens the bundle, never
    /// the gate. The market stays a stranger to both, and the non-TacSim roster's two
    /// pairs are unmoved ([`e7c3_wiring_the_quantity_delegation_is_declared_and_narrow`]).
    #[test]
    fn ic5_wiring_tacsim_is_the_third_quantity_delegate_on_the_game_roster() {
        use mush_testkit::TestWorld;
        let llm = std::sync::Arc::new(
            mush_llm::LlmService::builder()
                .global_enabled(false)
                .build(),
        );
        let roster = crate::boot::game_modules(&mush_config::config::Config::default(), &llm)
            .expect("the default config composes the game roster");
        let tw = TestWorld::with_modules(roster);
        let families = tw
            .engine()
            .registry
            .state_families()
            .expect("the composed state-family registry");
        let inventory = mush_core::OwnerId::new("inventory").expect("owner");
        let industry = mush_core::OwnerId::new("industry").expect("owner");
        let market = mush_core::OwnerId::new("market").expect("owner");
        let tacsim = mush_core::OwnerId::new("tacsim").expect("owner");
        assert_eq!(
            families.delegations(),
            vec![
                (industry.clone(), tacsim.clone()),
                (inventory.clone(), industry.clone()),
                (inventory.clone(), market.clone()),
                (inventory.clone(), tacsim.clone()),
                (tacsim.clone(), industry.clone()),
            ],
            "the game roster carries exactly five delegations: the three from `inventory` \
             outward and doc 20 cut SM.6's two between industry and tacsim"
        );
        assert!(
            families.delegates(&inventory, &tacsim),
            "inventory delegates its families to tacsim on the game roster"
        );
        assert!(
            !families.delegates(&tacsim, &inventory),
            "and the delegation is NOT symmetric"
        );
        assert!(
            families.delegates(&industry, &tacsim) && families.delegates(&tacsim, &industry),
            "doc 20 cut SM.6: industry and tacsim delegate to each other on the game roster"
        );
        assert!(
            !families.delegates(&tacsim, &market) && !families.delegates(&market, &tacsim),
            "tacsim and market are strangers in both directions"
        );
        assert!(
            !families.delegates(&industry, &market) && !families.delegates(&market, &industry),
            "industry and market are strangers in both directions"
        );
    }

    /// **Doc 18 cut IC8 — the game roster hands the layer a filled inventory cell**
    /// (lead 2c-iv): with `[tacsim].enabled`, `boot::game_modules` composes a layer
    /// whose `PortHandles` hold the inventory families, registers the production
    /// `tacsim_port_stock` definition, and answers the `market` fact namespace (the
    /// sixth, which a drawn sale's contract settles on).
    #[test]
    fn ic8_the_game_roster_hands_the_layer_a_filled_inventory_cell() {
        use mush_testkit::TestWorld;
        let llm = std::sync::Arc::new(
            mush_llm::LlmService::builder()
                .global_enabled(false)
                .build(),
        );
        let mut config = mush_config::config::Config::default();
        config.tacsim.enabled = true;
        let roster = crate::boot::game_modules(&config, &llm)
            .expect("an enabled config composes the game roster");
        let tw = TestWorld::with_modules(roster);
        let handles = tw
            .world()
            .module_state::<tacsim::port::PortHandles>()
            .expect("the port handles");
        assert!(
            handles.inventory.is_some(),
            "the layer holds the filled cell"
        );
        let definitions = tw.registry().definitions().expect("frozen definitions");
        let stock = mush_core::DefinitionKey {
            owner: mush_core::OwnerId::new(tacsim::NS).expect("owner"),
            kind: mush_core::DefinitionKind::new("action").expect("kind"),
            id: mush_core::DefinitionId::new(tacsim::port::STOCK_ACTION).expect("id"),
        };
        assert!(
            definitions.current(&stock).is_some(),
            "the production tacsim_port_stock definition is registered"
        );
        assert!(
            mush_economy::has_fact_provider(tw.world(), "market"),
            "the market fact namespace is answered"
        );
    }

    /// E5c-C231 (look-back fold), re-pointed by 16b cut R-b: the
    /// `[tacsim.economy].reserved_rooms` → `server_modules_with` →
    /// `HousingModule::with_reserved_rooms` chain. Housing's store is told every room
    /// the config names, plus `[onboarding]`'s two start rooms and Limbo.
    ///
    /// The `housing` knob still gates BOTH halves of the title issuer at once, which
    /// is the property `server_modules_with` computes `housing_on` once for: with it
    /// off, housing names no issuer and the economy lists none, so `@property/create`
    /// refuses in housing's own words rather than with a permission refusal.
    #[test]
    fn lb_e5cc231_the_reserved_rooms_reach_housing_through_server_modules_with() {
        use mush_testkit::TestWorld;
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig {
            reserved_rooms: vec![mush_core::ObjId(4_242), mush_core::ObjId(4_243)],
            ..tacsim::GameEconomyConfig::default()
        };
        assert!(game.housing, "housing is on at the game's defaults");
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            mush_campaign::RecoveryConfig::default(),
        ));
        let store = tw
            .world()
            .module_state::<mush_housing::HousingStore>()
            .expect("housing is wired");
        for room in [
            mush_core::ObjId(4_242),
            mush_core::ObjId(4_243),
            onboarding.start_room,
            onboarding.start_room_unapproved,
            mush_core::LIMBO,
        ] {
            assert!(
                store.is_reserved(room),
                "#{} did not reach housing's reserved list",
                room.0
            );
        }
        assert!(
            !store.is_reserved(mush_core::ObjId(9_999)),
            "an unnamed room was reserved"
        );
        assert_eq!(
            store.issuer().map(|i| i.as_str().to_string()).as_deref(),
            Some(housing_title_issuer().as_str()),
            "the title issuer is wired with housing on"
        );
        // `[tacsim.economy].housing = false` reserves the host's rooms alone and wires
        // no issuer.
        let off = tacsim::GameEconomyConfig {
            housing: false,
            ..tacsim::GameEconomyConfig::default()
        };
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &off,
            mush_campaign::RecoveryConfig::default(),
        ));
        let store = tw
            .world()
            .module_state::<mush_housing::HousingStore>()
            .expect("housing is wired");
        assert!(store.is_reserved(mush_core::LIMBO) && store.is_reserved(onboarding.start_room));
        assert!(!store.is_reserved(mush_core::ObjId(4_242)));
        assert!(store.issuer().is_none());
    }

    /// SP1 (D-LBK-7): a cadence declared in `[economy.cadences]` reaches the
    /// composition-frozen clock registry through `economy_config` →
    /// `server_modules_with` → `EconomyModule::register`, is created at boot, and
    /// is read through `Engine::clock_now` as `Registered { economy, cadence.<name> }`.
    /// A layer booting after the economy finds the declared clock by name instead of
    /// minting its own manual one.
    ///
    /// 16b cut R-b: the cadence is **test-declared**. It used to be the WoD layer's
    /// own `chronicle.period`, asserted against `WodEconomyConfig::cadence`; the
    /// mechanism under test — declaration → registry → `clock_now` — never was the
    /// ruleset's, and the one game declares no cadence of its own today.
    #[test]
    fn sp1_a_declared_cadence_reaches_the_registry_through_server_modules_with() {
        use mush_testkit::TestWorld;
        let cfg: mush_config::config::Config = toml::from_str(
            "[economy.cadences.\"chronicle.period\"]\nclock = \"real_utc\"\n\
             period_secs = 2592000\n",
        )
        .expect("parses");
        let economy = crate::config_ext::economy_config(&cfg.economy).expect("converts");
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let mut mods = server_modules_with(
            mush_weather::WeatherConfig::default(),
            economy,
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            mush_campaign::RecoveryConfig::default(),
        );
        mods.extend(sheet_modules(&onboarding, &game));
        let tw = TestWorld::with_modules(mods);
        let clock = mush_core::ClockRef::Registered {
            owner: mush_core::OwnerId::new("economy").expect("owner"),
            id: mush_core::CanonicalId::new("cadence.chronicle.period").expect("id"),
            generation: 1,
        };
        let stamp = tw
            .engine()
            .clock_now(&clock)
            .expect("the declared cadence is registered and readable");
        assert_eq!(stamp.at, 0);
        assert_eq!(stamp.clock, clock);
        let store = tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .expect("economy");
        let c = mush_economy::cadence_by_name(store.snapshot(), "chronicle.period")
            .expect("created at boot");
        assert_eq!(
            (c.schema, c.source, c.period),
            (
                mush_economy::CADENCE_SCHEMA_DECLARED,
                mush_economy::CadenceSource::RealUtc,
                2_592_000
            )
        );
        assert_eq!(
            store.snapshot().cadences().count(),
            1,
            "exactly the declared one: no layer minted a manual cadence beside it"
        );
        // An undeclared id has no provider.
        let other = mush_core::ClockRef::Registered {
            owner: mush_core::OwnerId::new("economy").expect("owner"),
            id: mush_core::CanonicalId::new("cadence.undeclared.month").expect("id"),
            generation: 1,
        };
        assert!(matches!(
            tw.engine().clock_now(&other),
            Err(mush_core::ClockReadError::ProviderMissing { .. })
        ));
    }

    #[test]
    fn server_modules_register_without_collision() {
        // The full production list (including org/economy and sim/tactics) shares one
        // registry: registration must not collide, and every module must be present
        // in the load-bearing boot order.
        use mush_testkit::TestWorld;
        let tw = TestWorld::with_modules(server_modules());
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys",
                "canon",
                "privileges",
                "org",
                "faction",
                "scenes",
                "sim",
                "atlas",
                "tactics",
                // E7 cut 7: the propagation layer, immediately after
                // `mush-tactics`. Its boot reads no tactics state, but its
                // POLICY reads the leak-safe contact ledger tactics publishes
                // and its contexts are built off `mush-sim`'s placement graph,
                // both of which are hydrated by then (`on_world_loaded` runs in
                // list order).
                "propagation",
                "economy",
                // E7 cut 2: immediately after the economy (its boot asks about
                // vault status) and before the market (whose listing quantity is
                // its own advertised counter, never a `Pool` claim).
                "inventory",
                // E7 cut 3: immediately after `mush-inventory`, whose handles
                // and family delegation it needs, and before `mush-market`,
                // which it does not touch.
                "industry",
                // E7 cut 9: the campaign layer, after economy/org/canon — its
                // bond, its `force.*` authority and its root canon record are
                // theirs — and before the player-facing subsystems.
                "campaign",
                "mail",
                "jobs",
                "housing",
                "weather",
                "party",
                "roll",
                "market",
                "script"
            ]
        );
    }

    /// E7 cut 1 (10b §3.1, doc 10 §20): **the TacSim layer composes after the
    /// character sheet**, and its order position is what makes that true.
    ///
    /// `on_world_loaded` runs in list order, so registering last is what lets TacSim's
    /// boot read the sheet's facts while the sheet reads TacSim's environmental facts
    /// through the generic seam. `[tacsim].enabled` switches the layer's BEHAVIOUR and
    /// never its place in this list.
    ///
    /// W1-B7-FIX: the roster comes from [`ProductionRoster`] — the one root
    /// `boot::game_modules` — rather than being hand-assembled here. WS04-S8's
    /// claim is that exactly ONE hand-built roster survives, as the negative
    /// control; this test was a second one, which made the claim false and,
    /// worse, made *this* test's ordering proof a proof about a list no server
    /// composes. The engine is still built from the list, because the claim is
    /// that the order survives composition and not merely that
    /// `game_modules` returned it in that order.
    #[test]
    fn e7c1_wiring_tacsim_composes_after_the_character_ruleset() {
        use mush_testkit::TestWorld;
        let mut roster = ProductionRoster::build("e7c1");
        let tw = TestWorld::with_modules(roster.take());
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys",
                "canon",
                "privileges",
                "org",
                "faction",
                "scenes",
                "sim",
                "atlas",
                "tactics",
                "propagation",
                "economy",
                "inventory",
                // E7 cut 3: immediately after `mush-inventory`, whose handles
                // and family delegation it needs, and before `mush-market`,
                // which it does not touch.
                "industry",
                // E7 cut 9: the campaign layer, after economy/org/canon.
                "campaign",
                "mail",
                "jobs",
                "housing",
                "weather",
                "party",
                "roll",
                "market",
                "script",
                // The composition root's own two, which only `game_modules`
                // adds.
                "worldgen",
                "npc",
                // 16b cut R-b: the one game sheet, and its rules interface.
                "pilot",
                "rules_interface",
                "tacsim",
                // S1: and the content module after it — the boot pass that
                // drives every other module's appliers runs last of all.
                "content"
            ],
            "the TacSim layer must come after the character sheet — only the content \
             module composes after it"
        );
    }

    #[test]
    fn the_org_capability_vocabulary_covers_every_economy_capability() {
        // E3b2 §4.10: the wiring vocabulary is `mush_economy::CAPABILITIES` ∪
        // `mush_housing::CAPABILITIES` (E5b) ∪ the two org-administrative ids. If a
        // module starts asking about a capability the server does not wire,
        // `@org/grant` would refuse the very grant that module needs — so this is a
        // real coupling, pinned here.
        let vocab = org_capability_vocabulary();
        // M7 declares these exact scoped capabilities as vocabulary only;
        // their presence creates no grant or authorization.
        let registry_capabilities = [
            mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
            mush_roll::commerce::registry_recall::REGISTRY_RECALL_CAPABILITY,
            mush_roll::commerce::registry_revocation::REGISTRY_REVOCATION_CAPABILITY,
        ];
        // E7 cut 9 (OPEN 5): `mush_campaign::CAPABILITIES` joins them. A `force.*`
        // id missing here fails at REGISTRATION — `@org/grant` refuses it — rather
        // than silently at use, which is the whole point of the typo catcher.
        // 17g C1b: the named launch's governed hardware capabilities join them,
        // so the launch operator's `@org/grant`s on the importer's scopes land.
        for c in mush_economy::CAPABILITIES
            .iter()
            .chain(mush_housing::CAPABILITIES)
            .chain(mush_campaign::CAPABILITIES)
            .chain(registry_capabilities.iter())
            .chain(NAMED_LAUNCH_HARDWARE_CAPABILITIES.iter())
        {
            let id = mush_org::CapabilityId::new(c).expect("well-formed");
            assert!(vocab.contains(&id), "the wiring vocabulary omits {c}");
        }
        let administrative = [
            "org.role.manage",
            "org.grant.delegate",
            "relay.manage",
            "relay.inspect",
            "relay.publish",
            "relay.discover",
            "relay.use",
            "relay.speak",
        ];
        for c in administrative {
            assert!(
                vocab.contains(&mush_org::CapabilityId::new(c).expect("well-formed")),
                "{c}"
            );
        }
        assert_eq!(
            vocab.len(),
            mush_economy::CAPABILITIES.len()
                + mush_housing::CAPABILITIES.len()
                + mush_campaign::CAPABILITIES.len()
                + registry_capabilities.len()
                + NAMED_LAUNCH_HARDWARE_CAPABILITIES.len()
                + administrative.len()
        );
        // …and a `force.*` id that is NOT declared is absent, so a typo'd grant
        // still fails loudly rather than becoming an inert record.
        assert!(
            !vocab.contains(&mush_org::CapabilityId::new("force.comand").expect("well-formed"))
        );
        assert!(
            !vocab.contains(
                &mush_org::CapabilityId::new("economy.acount.post").expect("well-formed")
            )
        );
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
        mods.extend(sheet_modules(
            &mush_core::OnboardingConfig::default(),
            &tacsim::GameEconomyConfig::default(),
        ));
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys",
                "canon",
                "privileges",
                "org",
                "faction",
                "scenes",
                "sim",
                "atlas",
                "tactics",
                "propagation",
                "economy",
                "inventory",
                // E7 cut 3: immediately after `mush-inventory`, whose handles
                // and family delegation it needs, and before `mush-market`,
                // which it does not touch.
                "industry",
                // E7 cut 9: the campaign layer, after economy/org/canon — its
                // bond, its `force.*` authority and its root canon record are
                // theirs — and before the player-facing subsystems.
                "campaign",
                "mail",
                "jobs",
                "housing",
                "weather",
                "party",
                "roll",
                "market",
                "script",
                "worldgen",
                // 16b cut R-b: the one game sheet, and its rules interface.
                "pilot",
                "rules_interface"
            ]
        );
        assert!(tw.registry().spec_of("@gen").is_some());
        // S2b: the theme moved onto the atlas (`@atlas/theme`); `@theme` is gone.
        assert!(tw.registry().spec_of("@theme").is_none());
        assert!(tw.registry().spec_of("@atlas").is_some());
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
    fn ruleset_agent_grounds_a_pilot_npc_end_to_end() {
        // The generic grounding adapter carries the API-scaling claim end-to-end:
        // the NPC host dispatches through the `Ruleset` trait, so the game's own
        // sheet grounds its NPCs with no NPC-wiring of its own. 16b cut R-b
        // re-points it at the one sheet and its one ability; the seam is the same
        // one that used to carry `wod_roll` and `sr_roll`.
        use mush_npc::AgentGrounding;
        use mush_testkit::TestWorld;
        let mut mods = server_modules(); // carries the jobs module the sheet needs
        mods.extend(sheet_modules(
            &mush_core::OnboardingConfig::default(),
            &tacsim::GameEconomyConfig::default(),
        ));
        let mut tw = TestWorld::with_modules(mods);
        let npc = tw.spawn_player("Ganger");
        let agent = RulesetAgent::new(tacsim::sheet::PilotRuleset::new());
        // No sheet yet → dialogue-only (no profile).
        assert!(
            agent.profile(tw.world(), npc).is_none(),
            "an NPC with no sheet does not ground"
        );
        // The `@agent/sheet` bootstrap gives it a real pilot sheet via
        // Ruleset::init_sheet.
        agent
            .init_sheet(tw.world_mut(), npc, "", npc)
            .expect("init_sheet writes a sheet");
        // Now it grounds: a capability profile carrying the one `check` tool.
        let prof = agent
            .profile(tw.world(), npc)
            .expect("the NPC grounds through the generic Ruleset seam after init_sheet");
        assert!(
            prof.tools
                .iter()
                .any(|t| t.name == tacsim::sheet::agent::CHECK_ABILITY),
            "the grounded NPC exposes the check ability: {:?}",
            prof.tools
        );
        // …and the ability really runs on the world loop, against that NPC's own
        // sheet, through the same adapter the NPC module holds.
        let mut out = mush_core::Outbox::new();
        let outcome = agent
            .invoke_ability(
                tw.world_mut(),
                &mut out,
                npc,
                tacsim::sheet::agent::CHECK_ABILITY,
                serde_json::json!({ "rating": "gunnery", "difficulty": 7 }),
            )
            .expect("the one ability resolves");
        assert!(
            outcome.summary.contains("gunnery check"),
            "{}",
            outcome.summary
        );
        // Anything else is unknown: there is exactly one ability.
        assert!(matches!(
            agent.invoke_ability(
                tw.world_mut(),
                &mut out,
                npc,
                "wod_roll",
                serde_json::json!({}),
            ),
            Err(mush_ruleset::AbilityError::Unknown(_))
        ));
    }

    #[test]
    fn full_boot_list_registers_without_collision() {
        // The production concatenation `boot::game_modules` builds — built-ins
        // THEN the one game sheet — shares one registry with no collision, and
        // the sheet registers last (after jobs, so its approval payload handler
        // sees a loaded jobs module).
        use mush_testkit::TestWorld;
        let mut mods = server_modules();
        mods.extend(sheet_modules(
            &mush_core::OnboardingConfig::default(),
            &tacsim::GameEconomyConfig::default(),
        ));
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys",
                "canon",
                "privileges",
                "org",
                "faction",
                "scenes",
                "sim",
                "atlas",
                "tactics",
                "propagation",
                "economy",
                "inventory",
                // E7 cut 3: immediately after `mush-inventory`, whose handles
                // and family delegation it needs, and before `mush-market`,
                // which it does not touch.
                "industry",
                // E7 cut 9: the campaign layer, after economy/org/canon — its
                // bond, its `force.*` authority and its root canon record are
                // theirs — and before the player-facing subsystems.
                "campaign",
                "mail",
                "jobs",
                "housing",
                "weather",
                "party",
                "roll",
                "market",
                "script",
                // 16b cut R-b: the one game sheet, and its rules interface.
                "pilot",
                "rules_interface"
            ]
        );
        // The sheet's surface is live: +roll / +sheet / +chargen / +stat
        // registered, and the one panel feed present. The feed is named for the
        // SCHEMA's ruleset id (`tacsim`), not the module's name (`pilot`) —
        // `register_sheet_commands` reserves `<schema.ruleset>.sheet`.
        assert!(tw.registry().spec_of("+roll").is_some());
        assert!(tw.registry().spec_of("+sheet").is_some());
        assert!(tw.registry().spec_of("+chargen").is_some());
        assert!(tw.registry().spec_of("+stat").is_some());
        let feeds: Vec<&str> = tw
            .registry()
            .panel_feeds()
            .iter()
            .map(|(feed, _)| feed.as_str())
            .collect();
        assert!(feeds.contains(&"tacsim.sheet"), "{feeds:?}");
        // …and that is the sheet's ONLY feed. No `tacsim.roll`, and no
        // `tacsim.vitals`: doc 16 §3.3 leaves M32's `TacticalUnit` cells the one
        // damage authority, so the sheet declares no health track to publish.
        // (`tactics.vitals` is the tactical layer's own feed and is a different
        // module's.)
        for absent in ["tacsim.roll", "tacsim.vitals", "pilot.sheet"] {
            assert!(
                !feeds.contains(&absent),
                "{absent} must not be a feed: {feeds:?}"
            );
        }
    }
}
