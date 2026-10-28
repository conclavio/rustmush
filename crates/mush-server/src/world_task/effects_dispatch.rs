//! Off-loop external effect dispatch and the typed [`AsyncEffectRunner`] registry.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{Effect, Inbound, WorldCommand};
use mush_llm::LlmService;
use mush_store::ObjectStore;
use smol_str::SmolStr;
use tokio::sync::mpsc;

use super::WorldMsg;

/// Off-loop runner for a module's deferred [`Effect::ModuleAsync`] work.
pub trait AsyncEffectRunner: Send + Sync {
    /// The module namespace this runner serves (matching `Effect::ModuleAsync.module`).
    fn module(&self) -> &str;

    /// Build the off-loop future for a deferred module effect, or `None` to
    /// drop the effect when its required service is absent.
    fn run(
        &self,
        correlation: u64,
        payload: String,
        llm: Option<Arc<LlmService>>,
    ) -> Option<BoxFuture<'static, String>>;
}

/// Registry of typed [`AsyncEffectRunner`] implementations keyed by module namespace.
#[derive(Clone, Default)]
pub struct AsyncEffectRegistry {
    runners: BTreeMap<SmolStr, Arc<dyn AsyncEffectRunner>>,
}

impl AsyncEffectRegistry {
    /// Construct an empty effect runner registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one module's [`AsyncEffectRunner`].
    pub fn register(&mut self, runner: Arc<dyn AsyncEffectRunner>) {
        self.runners.insert(SmolStr::new(runner.module()), runner);
    }

    /// Look up the runner registered for `module`.
    #[must_use]
    pub fn get(&self, module: &str) -> Option<&Arc<dyn AsyncEffectRunner>> {
        self.runners.get(module)
    }

    /// The shared production [`AsyncEffectRegistry`] with all standard module runners installed.
    #[must_use]
    pub fn production() -> &'static Self {
        static PRODUCTION: OnceLock<AsyncEffectRegistry> = OnceLock::new();
        PRODUCTION.get_or_init(|| {
            let mut reg = Self::new();
            reg.register(Arc::new(WorldgenEffectRunner));
            reg.register(Arc::new(NpcEffectRunner));
            reg.register(Arc::new(AtlasEffectRunner));
            reg.register(Arc::new(TacsimEffectRunner));
            reg.register(Arc::new(WeatherEffectRunner));
            reg
        })
    }
}

struct WorldgenEffectRunner;

impl AsyncEffectRunner for WorldgenEffectRunner {
    fn module(&self) -> &str {
        mush_worldgen::NS
    }

    fn run(
        &self,
        correlation: u64,
        payload: String,
        llm: Option<Arc<LlmService>>,
    ) -> Option<BoxFuture<'static, String>> {
        let Some(llm) = llm else {
            tracing::warn!(
                correlation,
                "worldgen Effect::ModuleAsync dropped: no LlmService on this world task"
            );
            return None;
        };
        Some(Box::pin(async move {
            mush_worldgen::run_worldgen_async(&llm, &payload).await
        }))
    }
}

struct NpcEffectRunner;

impl AsyncEffectRunner for NpcEffectRunner {
    fn module(&self) -> &str {
        mush_npc::NS
    }

    fn run(
        &self,
        correlation: u64,
        payload: String,
        llm: Option<Arc<LlmService>>,
    ) -> Option<BoxFuture<'static, String>> {
        let Some(llm) = llm else {
            tracing::warn!(
                correlation,
                "npc Effect::ModuleAsync dropped: no LlmService on this world task"
            );
            return None;
        };
        Some(Box::pin(async move {
            mush_npc::run_npc_turn_async(&llm, &payload).await
        }))
    }
}

struct AtlasEffectRunner;

impl AsyncEffectRunner for AtlasEffectRunner {
    fn module(&self) -> &str {
        mush_atlas::NS
    }

    fn run(
        &self,
        _correlation: u64,
        payload: String,
        _llm: Option<Arc<LlmService>>,
    ) -> Option<BoxFuture<'static, String>> {
        Some(Box::pin(async move {
            tokio::task::spawn_blocking(move || mush_atlas::run_async(&payload))
                .await
                .unwrap_or_else(|error| {
                    format!(
                        "{{\"domain\":0,\"revision\":0,\"result\":{{\"Err\":\"the compositor thread failed: {error}\"}}}}"
                    )
                })
        }))
    }
}

struct TacsimEffectRunner;

impl AsyncEffectRunner for TacsimEffectRunner {
    fn module(&self) -> &str {
        tacsim::store::NS
    }

    fn run(
        &self,
        correlation: u64,
        payload: String,
        llm: Option<Arc<LlmService>>,
    ) -> Option<BoxFuture<'static, String>> {
        let Some(llm) = llm else {
            tracing::warn!(
                correlation,
                "tacsim Effect::ModuleAsync: no LlmService on this world task; the \
                 squad stays on doctrine"
            );
            let refusal = tacsim::deliberation::unavailable_reply(&payload);
            return Some(Box::pin(async move { refusal }));
        };
        Some(Box::pin(async move {
            tacsim::deliberation::run_deliberation_async(&llm, &payload).await
        }))
    }
}

struct WeatherEffectRunner;

impl AsyncEffectRunner for WeatherEffectRunner {
    fn module(&self) -> &str {
        mush_weather::NS
    }

    fn run(
        &self,
        correlation: u64,
        payload: String,
        llm: Option<Arc<LlmService>>,
    ) -> Option<BoxFuture<'static, String>> {
        let Some(llm) = llm else {
            tracing::warn!(
                correlation,
                "weather Effect::ModuleAsync dropped: no LlmService on this world task"
            );
            return None;
        };
        Some(Box::pin(async move {
            mush_weather::run_weather_async(&llm, &payload).await
        }))
    }
}

/// Spawn one detached task per drained [`Effect`] using the production
/// [`AsyncEffectRegistry`]; each completion re-enters the world as a typed
/// [`WorldCommand`] via [`Inbound::Apply`].
pub(super) fn spawn_effect_list(
    effects: Vec<Effect>,
    tx: &mpsc::WeakSender<WorldMsg>,
    llm: Option<&Arc<LlmService>>,
    store: &Arc<dyn ObjectStore>,
) {
    spawn_effect_list_with_registry(effects, tx, llm, store, AsyncEffectRegistry::production());
}

/// Spawn one detached task per drained [`Effect`] against an explicit
/// [`AsyncEffectRegistry`].
pub(super) fn spawn_effect_list_with_registry(
    effects: Vec<Effect>,
    tx: &mpsc::WeakSender<WorldMsg>,
    llm: Option<&Arc<LlmService>>,
    store: &Arc<dyn ObjectStore>,
    registry: &AsyncEffectRegistry,
) {
    for effect in effects {
        match effect {
            Effect::StoreRead {
                module,
                actor,
                correlation,
                keys,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(%module, correlation, "world channel closing; store read dropped");
                    continue;
                };
                let store = Arc::clone(store);
                tokio::spawn(async move {
                    let result =
                        crate::store_read::perform_store_read(&*store, module.as_str(), &keys)
                            .await;
                    let done = WorldCommand::StoreReadDone {
                        module,
                        actor,
                        correlation,
                        payload: result.to_payload(),
                    };
                    if tx
                        .send(WorldMsg::Inbound(Inbound::Apply(done)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(
                            correlation,
                            "store read completion after shutdown dropped"
                        );
                    }
                });
            }
            Effect::SlowTest {
                actor,
                correlation,
                seconds,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(correlation, "world channel closing; slowtest dropped");
                    continue;
                };
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                    let done = WorldCommand::SlowTestDone {
                        actor,
                        correlation,
                        text: format!(
                            "SLOWTEST: fake async call #{correlation} finished after \
                             {seconds}s -- the world loop never blocked."
                        ),
                    };
                    if tx
                        .send(WorldMsg::Inbound(Inbound::Apply(done)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(correlation, "slowtest completion after shutdown dropped");
                    }
                });
            }
            Effect::ModuleAsync {
                module,
                actor,
                correlation,
                payload,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(%module, correlation, "world channel closing; module-async dropped");
                    continue;
                };
                let Some(runner) = registry.get(module.as_str()) else {
                    tracing::warn!(
                        %module,
                        correlation,
                        "Effect::ModuleAsync for an unrouted module dropped"
                    );
                    continue;
                };
                let Some(fut) = runner.run(correlation, payload, llm.cloned()) else {
                    continue;
                };
                tokio::spawn(async move {
                    let result = fut.await;
                    let done = WorldCommand::ModuleAsyncDone {
                        module,
                        actor,
                        correlation,
                        payload: result,
                    };
                    if tx
                        .send(WorldMsg::Inbound(Inbound::Apply(done)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(
                            correlation,
                            "module-async completion after shutdown dropped"
                        );
                    }
                });
            }
        }
    }
}
