//! The real production Moved roster is versioned before completion can migrate.
use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::moved_observer::MovedObserverMode;
use mush_core::{Engine, WorldEvent};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn production() -> TestResult<Engine> {
    let mut config = mush_config::config::Config::default();
    config.tacsim.enabled = true;
    let llm = Arc::new(mush_llm::LlmService::from_config(
        &mush_llm::LlmConfig::default(),
    )?);
    Ok(Engine::with_modules(
        mush_core::World::new_minimal(),
        mush_server::world_task::base_registry(),
        mush_server::boot::game_modules(&config, &llm)?,
    )?)
}

#[test]
fn actual_production_observers_have_exact_distinct_ids_and_retain_dispatch_order() -> TestResult {
    let engine = production()?;
    let roster = engine.registry.moved_observer_roster()?;
    let definitions = engine.registry.definitions().ok_or("definitions")?;
    let mut names = BTreeSet::new();
    let mut owners = Vec::new();
    for entry in roster.entries() {
        let reference = entry
            .definition()
            .ok_or("anonymous production Moved observer")?;
        assert_eq!(&reference.key().owner, entry.owner());
        assert_eq!(reference.key().kind.as_str(), "moved_observer");
        assert_eq!(reference.version(), 1);
        definitions.inspect_exact(reference)?;
        assert_eq!(entry.mode(), MovedObserverMode::LegacyPostMutation);
        names.insert((entry.owner().as_str(), reference.key().id.as_str()));
        owners.push(entry.owner().as_str());
    }
    assert_eq!(
        names,
        BTreeSet::from([
            ("scenes", "scene_movement_transcript"),
            ("atlas", "travel_discovery"),
            ("atlas", "window_arrival"),
            ("npc", "movement_memory"),
            ("weather", "movement_weather"),
            ("tacsim", "ic_return_stamp"),
            ("tacsim", "tutorial_lobby_entry"),
            ("script", "movement_scripts"),
        ])
    );
    assert_eq!(roster.entries().len(), names.len());
    let actual = engine.registry.observers_matching(&WorldEvent::Moved {
        who: mush_core::GOD,
        from: None,
        to: mush_core::LIMBO,
    });
    assert_eq!(
        owners,
        actual
            .iter()
            .map(|(owner, _)| owner.as_str())
            .collect::<Vec<_>>()
    );
    // Stable identity is not a claim that any mutable callback is a prepared
    // durable adapter. Every one remains a live-migration blocker in this cut.
    assert!(roster.has_blockers());
    Ok(())
}

#[test]
fn independently_composed_production_rosters_have_identical_digests() -> TestResult {
    let first = production()?;
    let second = production()?;
    assert_eq!(
        first.registry.moved_observer_roster()?.digest(),
        second.registry.moved_observer_roster()?.digest()
    );
    Ok(())
}
