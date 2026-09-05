//! Rule-1 tripwire: the pinned legacy-handler allowlist (08i §4; K answer 12,
//! ratified 2026-09-04).
//!
//! Doc 09 §7 rule 1 — "no new mutating legacy handlers" — is ratified with a
//! narrower exception than 08i Draft 0.1 proposed: allowed temporarily are bounded
//! whole-document workflow/catalog records with strict decoding, deterministic
//! keys, a declared future family or migration, and no participation in authority
//! or prepared mechanics; not exempt are reservations, contacts, leases, windows,
//! production state, mechanical offers, or anything that changes action legality.
//! The economy carve-out applies where the Ledger/seal gates prevent migration,
//! not automatically to every E6/E7 handler. K8.5 replaces the allowlist with
//! command registration metadata (read-only / legacy-post-image / authoritative).
//!
//! Until then `legacy-handlers.toml` pins the rule: every command the production
//! server registers is listed with one category; the verbs that existed at the pin
//! are `baseline` (deliberately not hand-classified here — that is K8.5's
//! metadata); every handler added since must be listed, in the same commit, with a
//! non-baseline category, a note saying why it is exempt or which prepared path it
//! rides, and, for `legacy-post-image`, its future family.
//!
//! The registry under test is the production composition, built the way
//! `mush-server`'s `build_engine` builds it: the core and engine registry, the
//! server's staff commands, `server_modules_with`, worldgen, the NPC module and a
//! ruleset — once per ruleset, because rustmush runs one ruleset per server and
//! `wod`/`shadowrun` collide on `+roll`/`+sheet`/`+chargen`. "Every ruleset
//! enabled" is therefore the union over every single-ruleset composition, with
//! default configuration (`[rulesets].enabled` is the only knob that changes
//! which verbs exist).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use mush_core::Engine;
use mush_server::config_ext;
use mush_server::modules;
use mush_server::staff::{StaffQueue, register_server_commands};
use mush_server::world_task::base_registry;
use serde::Deserialize;

/// The checked-in allowlist, beside this crate's `Cargo.toml`.
const ALLOWLIST: &str = "legacy-handlers.toml";

/// The five categories, exactly (K answer 12 plus the pin's own `baseline`).
const CATEGORIES: [&str; 5] = [
    "baseline",
    "read-only",
    "legacy-post-image",
    "authoritative",
    "economy-carve-out",
];

/// Every ruleset a production server can enable (`mush_server::modules::ruleset_modules`).
const RULESETS: [&str; 2] = ["wod", "shadowrun"];

/// How many `baseline` entries the pin carries — every command registered on
/// `main` at `b0e3b52`. A baseline entry may be **retired** with its verb (lower
/// this number in the same commit); one may never be **added**: a new handler is
/// listed with a non-baseline category.
const PINNED_BASELINE: usize = 159;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    #[serde(default)]
    handler: Vec<Handler>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Handler {
    /// The primary verb, exactly as registered (`CommandSpec::name`; aliases are
    /// not separate handlers).
    name: String,
    category: String,
    /// The future typed family — required for `legacy-post-image`.
    #[serde(default)]
    family: Option<String>,
    /// Why the handler is exempt, or which prepared path it rides — required for
    /// every non-baseline entry (a baseline note is provenance only).
    #[serde(default)]
    note: Option<String>,
}

/// The primary verb of every command the production server registers with
/// `[rulesets].enabled = [ruleset]` and default configuration, composed exactly
/// as `build_engine` composes it (help topics are prose, not commands, and are
/// not loaded).
fn production_commands(ruleset: &str) -> BTreeSet<String> {
    let config = mush_config::config::Config::default();
    let enabled = vec![ruleset.to_string()];
    let staff = Arc::new(StaffQueue::new());
    let mut registry = base_registry();
    register_server_commands(&mut registry, &staff)
        .expect("server command registration must not collide");
    // The worldgen module wants an LLM service; no provider is configured and the
    // service is disabled, which changes nothing about which verbs register.
    let llm = Arc::new(
        mush_llm::LlmService::builder()
            .global_enabled(false)
            .build(),
    );
    let onboarding = config_ext::onboarding_ruleset(&config.onboarding);
    let wod_economy =
        config_ext::wod_economy_config(&config.wod).expect("the default [wod.economy] converts");
    let wod_selected = enabled.iter().any(|r| r.eq_ignore_ascii_case("wod"));
    let sr_economy = config_ext::shadowrun_economy_config(&config.shadowrun)
        .expect("the default [shadowrun.economy] converts");
    let sr_selected = enabled.iter().any(|r| r.eq_ignore_ascii_case("shadowrun"));
    let mut mods = modules::server_modules_with(
        config_ext::weather_config(&config.weather),
        config_ext::economy_config(&config.economy).expect("the default [economy] converts"),
        &onboarding,
        wod_selected.then_some(&wod_economy),
        sr_selected.then_some(&sr_economy),
    );
    mods.push(modules::worldgen_module_with(
        Arc::clone(&llm),
        config_ext::worldgen_config(&config.worldgen),
    ));
    mods.push(modules::npc_module(modules::npc_agent_adapter(&enabled)));
    mods.extend(
        modules::ruleset_modules(&enabled, &onboarding, &wod_economy, &sr_economy)
            .expect("a known ruleset composes"),
    );
    // `build_engine`'s own step: compose and freeze every registry before any world
    // record exists. The verbs are fixed here; no world and no boot are needed.
    let composition = Engine::compose_modules(registry, mods)
        .expect("the production composition registers without collision");
    composition
        .registry()
        .specs()
        .map(|spec| spec.name.to_string())
        .collect()
}

/// The union over every single-ruleset production composition.
fn registered_commands() -> BTreeSet<String> {
    RULESETS
        .iter()
        .flat_map(|r| production_commands(r))
        .collect()
}

fn stanza_to_add(name: &str) -> String {
    format!(
        "\n[[handler]]\nname = {name:?}\n\
         category = \"<read-only | legacy-post-image | authoritative | economy-carve-out>\"\n\
         family = \"<the future typed family; required for legacy-post-image>\"\n\
         note = \"<why it is exempt, or which prepared path it rides>\"\n"
    )
}

#[test]
fn every_registered_command_is_pinned_in_the_allowlist_with_a_category() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let list: Allowlist = toml::from_str(&text)
        .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
    let registered = registered_commands();
    assert!(
        !registered.is_empty(),
        "the production composition registered no commands"
    );

    let mut problems: Vec<String> = Vec::new();
    let mut listed: BTreeSet<&str> = BTreeSet::new();
    let mut previous: Option<&str> = None;
    let mut baseline = 0usize;
    for handler in &list.handler {
        let name = handler.name.as_str();
        if !CATEGORIES.contains(&handler.category.as_str()) {
            problems.push(format!(
                "{name:?}: unknown category {:?} (exactly one of {})",
                handler.category,
                CATEGORIES.join(" | ")
            ));
        }
        if handler.category == "legacy-post-image"
            && handler
                .family
                .as_deref()
                .is_none_or(|family| family.trim().is_empty())
        {
            problems.push(format!(
                "{name:?}: a legacy-post-image handler must name its future family \
                 (family = \"<owner.family>\")"
            ));
        }
        if handler.category == "baseline" {
            baseline += 1;
        } else if handler
            .note
            .as_deref()
            .is_none_or(|note| note.trim().is_empty())
        {
            problems.push(format!(
                "{name:?}: a non-baseline handler must say why it is exempt or which prepared \
                 path it rides (note = \"...\")"
            ));
        }
        if !listed.insert(name) {
            problems.push(format!("{name:?} is listed more than once"));
        }
        if previous.is_some_and(|prev| prev > name) {
            problems.push(format!(
                "{name:?} is out of order: entries are sorted by name in byte order, and \
                 {:?} precedes it",
                previous.unwrap_or_default()
            ));
        }
        previous = Some(name);
        if !registered.contains(name) {
            problems.push(format!(
                "{name:?} is listed but no production composition registers it \
                 (retired? remove its stanza)"
            ));
        }
    }

    let missing: Vec<&str> = registered
        .iter()
        .map(String::as_str)
        .filter(|name| !listed.contains(name))
        .collect();
    if !missing.is_empty() {
        let stanzas: String = missing.iter().map(|name| stanza_to_add(name)).collect();
        problems.push(format!(
            "{} registered command(s) are not in {ALLOWLIST}. Rule 1 as ratified (08i §4, K \
             answer 12): add each in this commit with a NON-baseline category, and name the \
             future family for legacy-post-image:{stanzas}",
            missing.len()
        ));
    }

    if baseline != PINNED_BASELINE {
        problems.push(format!(
            "{baseline} baseline entries, but the pin at b0e3b52 carries {PINNED_BASELINE}: a \
             baseline entry may be retired with its verb (lower PINNED_BASELINE in this \
             commit); a new handler is never baseline — give it a real category"
        ));
    }

    assert!(
        problems.is_empty(),
        "rule-1 allowlist tripwire ({}):\n- {}",
        path.display(),
        problems.join("\n- ")
    );
}

/// The pinned file's own consistency: the union is exactly what the two
/// compositions register, and neither composition is empty — so a ruleset that
/// stops composing cannot silently shrink the surface the tripwire watches.
#[test]
fn every_single_ruleset_composition_contributes_to_the_union() {
    let union = registered_commands();
    for ruleset in RULESETS {
        let names = production_commands(ruleset);
        assert!(!names.is_empty(), "{ruleset}: no commands registered");
        assert!(
            names.is_subset(&union),
            "{ruleset}: the union misses one of its commands"
        );
    }
}
