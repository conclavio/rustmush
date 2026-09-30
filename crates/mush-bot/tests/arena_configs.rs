//! **The example arena configs load** (cut AJ.4): `bots/arena/pilot.toml` and
//! `bots/arena/commander.toml`, read by the bot's own `Config::load` exactly as
//! `--config` reads them, validate, and say what `docs/bot.md` says they say.
//! Offline: nothing here resolves a credential or opens a connection, and no
//! model is asked anything.
use std::path::PathBuf;

use mush_bot::{arena::settings::Settings, config::Config};
use serde_json::json;

/// A file under the repository's `bots/arena/`.
fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../bots/arena")
        .join(name)
}

#[test]
fn the_example_arena_configs_load() {
    // The pilot: no model of any kind, every `[arena]` key at its default, no
    // memory across runs.
    let pilot = Config::load(&example("pilot.toml")).expect("pilot.toml loads and validates");
    pilot.validate().expect("pilot.toml validates");
    assert!(pilot.model.is_none(), "the pilot needs no [model]");
    assert!(pilot.arena.commander_model.is_none());
    assert!(!pilot.arena.commander_from_session);
    assert_eq!(pilot.arena, Settings::default(), "every key at its default");
    assert!(!pilot.memory.persistent);
    assert_eq!(pilot.memory.namespace, "arena");
    assert!(pilot.costs.persistent);
    assert!(pilot.agent.bootstrap.is_empty());
    assert_eq!(pilot.agent.max_seconds, 900);
    assert_eq!(pilot.ssh.password_env.as_deref(), Some("MUSH_BOT_PASSWORD"));

    // The commander: the pilot's settings plus its own model table, routed as
    // the programme lead measured, and never the session's model.
    let commander =
        Config::load(&example("commander.toml")).expect("commander.toml loads and validates");
    commander.validate().expect("commander.toml validates");
    assert!(
        commander.model.is_none(),
        "no session model to fall back on"
    );
    assert!(commander.arena.commander);
    assert_eq!(commander.arena.commander_ms, 1_500);
    assert!(!commander.arena.commander_from_session);
    assert!(!commander.memory.persistent);
    assert_eq!(commander.memory.namespace, "arena");
    assert_eq!(
        Settings {
            commander_model: None,
            ..commander.arena.clone()
        },
        Settings::default(),
        "the rest of [arena] at its defaults"
    );
    let model = commander
        .arena
        .commander_model
        .as_ref()
        .expect("[arena.commander_model]");
    assert_eq!(model.name, "deepseek/deepseek-v4.1-flash");
    assert_eq!(
        model.endpoint,
        "https://openrouter.ai/api/v1/chat/completions"
    );
    assert_eq!(model.api_key_env.as_deref(), Some("OPENROUTER_API_KEY"));
    assert_eq!(model.timeout_secs, 5);
    assert_eq!(model.max_tokens, 200);
    assert_eq!(model.temperature, Some(0.2));
    assert_eq!(
        model.reasoning.clone().map(serde_json::Value::Object),
        Some(json!({"enabled": false}))
    );
    assert_eq!(
        model.provider.clone().map(serde_json::Value::Object),
        Some(json!({"order": ["Together"], "allow_fallbacks": false}))
    );

    // The commander resolves from the table, never from a session model.
    let resolved =
        mush_bot::arena::commander::resolve(&commander.arena, None).expect("a commander");
    assert!(resolved.from_table);
    assert_eq!(
        mush_bot::arena::commander::start_line(Some(&resolved)),
        "Commander: deepseek/deepseek-v4.1-flash from [arena.commander_model]."
    );
    assert!(mush_bot::arena::commander::resolve(&pilot.arena, None).is_none());
}
