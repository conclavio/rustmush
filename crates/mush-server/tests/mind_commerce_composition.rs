//! Explicit host composition only: the real production roster, not test owners.
//! These tests claim registration and missing-setup behavior, not certification,
//! purchases or writer recovery. No commercial row is manufactured here.

use std::sync::Arc;

use mush_config::config::Config;
use mush_core::{Engine, GameModule, ModError, World};
use mush_economy::registry_filing::InitialFilingPolicy;
use mush_economy::{AccountId, Amount, CatalogRef, EconomyStore, InstrumentId};
use mush_llm::LlmService;
use mush_market::{
    MarketId,
    mind_trade::{OriginalDuration, OriginalPrice, OriginalQuotePolicy, records::QuoteVenue},
};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_roll::commerce::{filing::InitialFilingTerms, module::CommerceMode};
use mush_server::{boot, config_ext, modules, persist, world_task};
use mush_store::{InMemoryStore, ObjectStore};
use tacsim::{TacsimRuleset, composition::mind_commerce::MindCommerce};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn id(value: &str) -> Result<SystemPrincipalId, ModError> {
    SystemPrincipalId::new(value).ok_or_else(|| ModError::msg("valid fixture issuer"))
}
fn config() -> Config {
    let mut config = Config::default();
    config.tacsim.enabled = true;
    config.store.store = "memory".into();
    config
}
fn llm(config: &Config) -> Result<Arc<LlmService>, String> {
    let llm = config_ext::llm_config(&config.llm)?;
    LlmService::from_config(&llm)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}
fn prepared(
    config: &Config,
    mode: CommerceMode,
    issuer: SystemPrincipalId,
) -> Result<MindCommerce, ModError> {
    let cfg = config_ext::tacsim_config(&config.tacsim).map_err(ModError::msg)?;
    // Exact same real layer/content choice as boot::game_modules. The public
    // preparation privately applies the actual shipped Atlas ground.
    let layer = TacsimRuleset::with_game_content(cfg.clone(), cfg.enabled);
    MindCommerce::prepare(
        &layer,
        mode,
        InitialFilingTerms::new(90 * 86_400, 3 * 86_400)
            .map_err(|error| ModError::msg(error.to_string()))?,
        InitialFilingPolicy::new(
            issuer,
            AccountId(1),
            Amount::from_minor(100),
            CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
    )
}
fn purchase_policy() -> Result<OriginalQuotePolicy, ModError> {
    let issuer = id("roll.registry")?;
    OriginalQuotePolicy::new(
        OriginalPrice::new(
            Amount::from_minor(100),
            InstrumentId::new("cr").ok_or_else(|| ModError::msg("currency"))?,
            issuer.clone(),
            AccountId(1),
            100,
        )?,
        QuoteVenue::new(
            MarketId::new("registry").ok_or_else(|| ModError::msg("venue"))?,
            PrincipalId::System(issuer),
            AccountId(1),
            0,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        OriginalDuration::new(true, 90 * 86_400, 3 * 86_400, 600)?,
    )
}
fn names(modules: &[Box<dyn GameModule>]) -> Vec<&str> {
    modules.iter().map(|module| module.name()).collect()
}
fn has(engine: &Engine, owner: &str, kind: &str, id: &str) -> bool {
    engine.registry.definitions().is_some_and(|definitions| {
        definitions.iter().any(|entry| {
            let key = entry.reference().key();
            key.owner.as_str() == owner && key.kind.as_str() == kind && key.id.as_str() == id
        })
    })
}
fn register(modules: Vec<Box<dyn GameModule>>) -> Result<Engine, ModError> {
    Engine::with_modules(World::new_minimal(), world_task::base_registry(), modules)
}

#[test]
fn full_roster_opt_in_preserves_order_and_default_definitions() -> TestResult {
    let config = config();
    let llm = llm(&config)?;
    let ordinary = boot::game_modules(&config, &llm)?;
    let expected: Vec<String> = names(&ordinary).into_iter().map(str::to_owned).collect();
    let ordinary = register(ordinary)?;
    for (owner, kind, id) in [
        ("tacsim", "registry_manifest", "combat"),
        ("roll", "action", "rating_admit"),
        ("roll", "action", "initialize_commerce"),
        ("market", "original_purchase_binding", "roll"),
    ] {
        assert!(!has(&ordinary, owner, kind, id));
    }
    for (mode, paid) in [
        (CommerceMode::Initialize, false),
        (CommerceMode::Active, false),
        (CommerceMode::Active, true),
    ] {
        let commerce = prepared(&config, mode, id("roll.registry")?)?;
        let commerce = if paid {
            commerce.with_original_purchases(purchase_policy()?)?
        } else {
            commerce
        };
        let modules = boot::game_modules_with_mind_commerce(&config, &llm, &commerce)?;
        assert_eq!(names(&modules), expected);
        let order = names(&modules);
        let position = |name| {
            order
                .iter()
                .position(|actual| *actual == name)
                .ok_or("missing owner")
        };
        assert!(position("economy")? < position("market")?);
        assert!(position("market")? < position("roll")?);
        assert!(position("roll")? < position("tacsim")?);
        assert_eq!(order.last(), Some(&"content"));
        let engine = register(modules)?;
        assert!(has(&engine, "tacsim", "registry_manifest", "combat"));
        assert_eq!(
            has(&engine, "roll", "action", "rating_admit"),
            mode == CommerceMode::Active
        );
        assert_eq!(
            has(&engine, "roll", "action", "initialize_commerce"),
            mode == CommerceMode::Initialize
        );
        assert_eq!(
            has(&engine, "market", "original_purchase_binding", "roll"),
            paid
        );
        assert!(
            engine
                .registry
                .definitions()
                .ok_or("definitions")?
                .iter()
                .any(|entry| entry.reference() == commerce.manifest())
        );
    }
    Ok(())
}

#[tokio::test]
async fn opt_in_adds_only_selected_issuer_and_preserves_existing_issuer_flags() -> TestResult {
    for flags in [false, true] {
        let mut config = config();
        config.tacsim.economy.housing = flags;
        config.tacsim.economy.market = flags;
        let llm = llm(&config)?;
        let ordinary_store = InMemoryStore::new();
        let mut ordinary = hydrated(boot::game_modules(&config, &llm)?, &ordinary_store).await?;
        modules::boot_modules(&mut ordinary, &ordinary_store).await?;
        for selected in [id("roll.registry")?, mush_industry::industry_title_issuer()] {
            let commerce = prepared(&config, CommerceMode::Initialize, selected.clone())?;
            let actual_store = InMemoryStore::new();
            let mut actual = hydrated(
                boot::game_modules_with_mind_commerce(&config, &llm, &commerce)?,
                &actual_store,
            )
            .await?;
            // Economy installs the frozen issuer table after journal replay.
            // Registration intentionally contains only an unbooted default slot.
            modules::boot_modules(&mut actual, &actual_store).await?;
            let before = ordinary
                .world
                .module_state::<EconomyStore>()
                .ok_or("economy")?;
            let after = actual
                .world
                .module_state::<EconomyStore>()
                .ok_or("economy")?;
            assert!(after.is_title_issuer(&selected));
            for issuer in [
                modules::housing_title_issuer(),
                modules::market_title_issuer(),
                mush_industry::industry_title_issuer(),
                id("tacsim.registry")?,
                id("roll.registry")?,
                id("unselected.registry")?,
            ] {
                assert_eq!(
                    after.is_title_issuer(&issuer),
                    before.is_title_issuer(&issuer) || issuer == selected,
                    "{issuer:?}"
                );
            }
            assert_eq!(
                after.is_title_issuer(&modules::housing_title_issuer()),
                flags
            );
            assert_eq!(
                after.is_title_issuer(&modules::market_title_issuer()),
                flags
            );
        }
    }
    Ok(())
}

#[test]
fn full_roster_refuses_stale_policy_and_disabled_layer() -> TestResult {
    let mut config = config();
    let llm = llm(&config)?;
    let commerce = prepared(&config, CommerceMode::Active, id("roll.registry")?)?;
    config.tacsim.match_.ai_fill = !config.tacsim.match_.ai_fill;
    assert!(boot::game_modules_with_mind_commerce(&config, &llm, &commerce).is_err());
    config.tacsim.match_.ai_fill = !config.tacsim.match_.ai_fill;
    config.tacsim.enabled = false;
    assert!(boot::game_modules_with_mind_commerce(&config, &llm, &commerce).is_err());
    // Explicit policy never changes the ordinary disabled-layer path.
    let default = register(boot::game_modules(&config, &llm)?)?;
    assert!(!has(&default, "tacsim", "registry_manifest", "combat"));
    Ok(())
}

async fn hydrated(
    modules: Vec<Box<dyn GameModule>>,
    store: &dyn ObjectStore,
) -> Result<Engine, String> {
    let composition = Engine::compose_modules(world_task::base_registry(), modules)
        .map_err(|error| error.to_string())?;
    let world = persist::load_or_seed_world(store).await?;
    let mut engine = Engine::from_composition(world, composition);
    let identity: Arc<dyn mush_core::AccountIdentity> =
        Arc::new(mush_server::accounts_identity::hydrate(store).await?);
    engine.set_account_identity(Arc::clone(&identity));
    let oracle = engine
        .world
        .module_state_mut::<mush_sim::AccountOracle>()
        .ok_or("actual Sim oracle")?;
    *oracle = mush_sim::AccountOracle::new(identity);
    Ok(engine)
}

#[tokio::test]
async fn actual_module_boot_cannot_turn_configuration_into_acknowledged_setup() -> TestResult {
    let config = config();
    let llm = llm(&config)?;
    let store = InMemoryStore::new();
    let initialize = prepared(&config, CommerceMode::Initialize, id("roll.registry")?)?;
    let modules = boot::game_modules_with_mind_commerce(&config, &llm, &initialize)?;
    let mut engine = hydrated(modules, &store).await?;
    // Real migrations, typed partitioning, all owner hooks and durable ordinary
    // boot writes. No setup action was submitted or acknowledged.
    modules::boot_modules(&mut engine, &store).await?;
    let before = store.module_json_list("roll").await?;
    assert!(
        !before
            .iter()
            .any(|(key, _)| key.starts_with("action-state/"))
    );
    drop(engine);
    let active = prepared(&config, CommerceMode::Active, id("roll.registry")?)?;
    let modules = boot::game_modules_with_mind_commerce(&config, &llm, &active)?;
    let mut engine = hydrated(modules, &store).await?;
    let error = modules::boot_modules(&mut engine, &store)
        .await
        .expect_err("Active requires an actual setup receipt");
    assert_eq!(error, "module boot failed: commercial record is missing");
    let mut after = store.module_json_list("roll").await?;
    let mut before = before;
    before.sort_by(|a, b| a.0.cmp(&b.0));
    after.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        after, before,
        "refused Active boot cannot synthesize deployment"
    );
    Ok(())
}
