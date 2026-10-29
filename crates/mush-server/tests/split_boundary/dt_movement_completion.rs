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

#[test]
fn actual_scenes_adapter_covers_only_its_callback_in_the_complete_production_manifest() -> TestResult
{
    use mush_core::locomotion::completion::{MovementCoverage, MovementManifestSnapshot};
    let engine = production()?;
    let definitions = engine.registry.definitions().ok_or("definitions")?;
    let action = definitions
        .iter()
        .find(|entry| {
            let key = entry.reference().key();
            key.owner.as_str() == "sim"
                && key.kind.as_str() == "action"
                && key.id.as_str() == mush_core::locomotion::scene_transition::ACTION
        })
        .ok_or("joint action")?
        .reference();
    let frozen = engine
        .registry
        .movement_completion_manifest(action)
        .ok_or("joint manifest")?;
    let snapshot = frozen.snapshot();
    let roster = engine.registry.moved_observer_roster()?;
    assert_eq!(snapshot.action(), action);
    assert_eq!(snapshot.roster_digest(), roster.digest());
    assert_eq!(snapshot.entries().len(), 8);
    assert_eq!(snapshot.groups().len(), 1);
    assert_eq!(snapshot.groups()[0].owner().as_str(), "scenes");
    assert_eq!(
        snapshot.groups()[0].participant().key().id.as_str(),
        "movement_transcript"
    );
    assert!(snapshot.has_blockers());
    for (index, (entry, actual)) in snapshot.entries().iter().zip(roster.entries()).enumerate() {
        assert_eq!(usize::from(entry.ordinal()), index);
        assert_eq!(entry.owner(), actual.owner());
        assert_eq!(entry.observer(), actual.definition());
        assert_eq!(
            entry.coverage(),
            if entry.owner().as_str() == "scenes" {
                MovementCoverage::Prepared { group: 0 }
            } else {
                MovementCoverage::LegacyBlocker
            }
        );
    }
    let retained = MovementManifestSnapshot::decode(&snapshot.encode()?, definitions)?;
    assert_eq!(&retained, snapshot);
    assert_eq!(retained.digest()?, frozen.digest());
    Ok(())
}

#[test]
fn composed_action_resolves_the_distinct_actual_scenes_group_and_retains_other_blockers()
-> TestResult {
    let engine = production()?;
    let definitions = engine.registry.definitions().ok_or("definitions")?;
    let action = definitions
        .iter()
        .find(|entry| {
            entry.reference().key().owner.as_str() == "sim"
                && entry.reference().key().kind.as_str() == "action"
                && entry.reference().key().id.as_str() == mush_core::locomotion::composed::ACTION
        })
        .ok_or("composed action")?
        .reference();
    let snapshot = engine
        .registry
        .movement_completion_manifest(action)
        .ok_or("manifest")?
        .snapshot();
    assert_eq!(snapshot.entries().len(), 8);
    assert_eq!(snapshot.groups().len(), 1);
    assert_eq!(snapshot.groups()[0].ordinals(), &[0]);
    assert_eq!(
        snapshot.groups()[0].participant().key().id.as_str(),
        mush_scenes::movement_composed::PARTICIPANT
    );
    assert!(snapshot.has_blockers());
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .filter(|entry| entry.coverage()
                == mush_core::locomotion::completion::MovementCoverage::LegacyBlocker)
            .count(),
        7
    );
    Ok(())
}
