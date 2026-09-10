//! WS03-A1: cross-namespace module writes, censused.
//!
//! A module's [`GameModule::name`] **is** its persistence namespace, so
//! `op.ns != ctx.module` at `Ctx::persist` is one module writing documents
//! another module owns. Until WS03-A1 that was unverified by construction —
//! the doc comment said so in as many words — which meant the delegation set
//! was not merely unpoliced but *unknown*.
//!
//! This is the census half, in the `state_slot_owners.rs` /
//! `legacy_handler_allowlist.rs` style: a checked-in file beside this crate's
//! `Cargo.toml`, a scan that fails on an undeclared hit, and a message that
//! names the file to edit. Nothing is refused yet. Two claims:
//!
//! 1. **Ownership resolves.** Every module of every production composition
//!    claims its own namespace, and every claimed namespace belongs to a
//!    module that actually composed. The composition is the one root both
//!    binaries boot — `mush_server::boot::game_modules` — reached the way
//!    `legacy_handler_allowlist.rs` reaches it (this crate has a `mush-server`
//!    dev-dependency and deliberately no `mushd` one: `mushd/src/boot.rs` is a
//!    delegator, and asserting against it would assert against a
//!    transcription).
//!
//! 2. **The census is written down.** Booting each production roster records
//!    every cross-namespace write the boot phase performs, and each one must
//!    carry a row in `module-ns-owners.toml`. The rows measured by the wider
//!    census RUN — the crate test suites executed with the counter installed,
//!    which reach code no boot does — are in the same file, with the route
//!    each takes.
//!
//! # What this can and cannot see
//!
//! Booting a roster exercises `on_world_loaded` and nothing else, so the boot
//! census is a floor, not the whole picture. The pairs that matter most are
//! the ones only play reaches (the jobs API, called under a producing
//! module's identity), and those were measured once by running the crate test
//! suites with the counter installed and written into the file by hand. That
//! is the honest instrument: `Ctx::persist`'s namespace is a runtime value,
//! not a syntactic one, so no scan of the source can enumerate it.
//!
//! `Ctx::persist_engine_owned` is `pub(crate)` in `mush-core` and is not a way
//! around any of this: it is the spelling for the writes the check would
//! misread — the scene API writing a scene's OWNING namespace whoever ends the
//! scene, and the world timeline writing its reserved engine-owned one.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use mush_core::engine::Effects;
use mush_core::{Engine, Outbox, World};
use mush_server::boot;
use mush_server::staff::{StaffQueue, register_server_commands};
use mush_server::world_task::base_registry;
use serde::Deserialize;
use smol_str::SmolStr;

/// The checked-in census, beside this crate's `Cargo.toml`.
const ALLOWLIST: &str = "module-ns-owners.toml";

/// Every ruleset a production server can enable, plus the ruleset-less
/// composition — `[rulesets].enabled` is the only knob that changes which
/// modules compose.
const RULESETS: [&str; 3] = ["", "wod", "shadowrun"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Census {
    /// Cross-namespace writes the census RUN observed, with the route each
    /// one takes.
    #[serde(default)]
    observed: Vec<Observed>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observed {
    /// The writing module, as `ctx.module` carries it.
    module: String,
    /// The namespace the write landed in (`op.ns`).
    ns: String,
    /// How a write gets from that module into that namespace.
    reason: String,
}

fn census() -> Census {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()))
}

/// A production server's engine with `[rulesets].enabled = [ruleset]` (or
/// nothing enabled, for `""`), built through the ONE composition root both
/// halves boot and then given its boot callback.
fn production_engine(ruleset: &str) -> Engine {
    let mut config = mush_config::config::Config::default();
    config.rulesets.enabled = if ruleset.is_empty() {
        Vec::new()
    } else {
        vec![ruleset.to_string()]
    };
    // No provider is configured and the service is disabled; which modules
    // compose does not depend on it.
    let llm = Arc::new(
        mush_llm::LlmService::builder()
            .global_enabled(false)
            .build(),
    );
    let mods = boot::game_modules(&config, &llm).expect("the default config composes a roster");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = base_registry();
    register_server_commands(&mut registry, &staff)
        .expect("server command registration must not collide");
    let composition = Engine::compose_modules(registry, mods)
        .expect("the production composition registers without collision");
    Engine::from_composition(World::new_minimal(), composition)
}

/// Every module name in one production composition.
fn module_names(engine: &Engine) -> BTreeSet<String> {
    engine
        .registry
        .namespace_owners()
        .into_iter()
        .map(|(_, module)| module.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// **Ownership resolves.** Composition records `namespace -> owning module`
/// for the whole roster: a module's name is its namespace, so every module
/// claims one, and nothing claims a namespace no module composed.
#[test]
fn every_production_module_claims_its_own_namespace() {
    for ruleset in RULESETS {
        let engine = production_engine(ruleset);
        let owners: Vec<(SmolStr, SmolStr)> = engine.registry.namespace_owners();
        assert!(
            owners.len() >= 20,
            "[{ruleset}] only {} claimed namespaces — the composition did not build",
            owners.len()
        );
        let modules = module_names(&engine);
        for (ns, module) in &owners {
            assert!(
                modules.contains(module.as_str()),
                "[{ruleset}] namespace {ns} is claimed by {module}, which is not a composed \
                 module — a migration namespace outlived its module?"
            );
        }
        // A module's own name is the namespace it owns; a namespace named
        // after no module belongs to whichever module registered its
        // migrations, and there are none of those today.
        let named_after_a_module: BTreeMap<&str, &str> = owners
            .iter()
            .filter(|(ns, module)| ns != module)
            .map(|(ns, module)| (ns.as_str(), module.as_str()))
            .collect();
        assert!(
            named_after_a_module.is_empty(),
            "[{ruleset}] namespaces claimed by a module of another name: \
             {named_after_a_module:?}. That is legal (Registrar::migrations' first-registrar \
             rule), but WS03-A1's refusal has to know about it — say so here in the same commit."
        );
    }
}

/// **Every cross-namespace write the BOOT phase performs is written down.**
///
/// `on_world_loaded` runs with the same module identity a command does, so a
/// module cleaning up another module's documents at boot is the same
/// cross-namespace write and gets no exemption. Today no module makes one:
/// the boot census is empty, and this is what keeps it that way.
#[test]
fn booting_a_production_roster_makes_no_undeclared_cross_namespace_write() {
    let declared: BTreeSet<(String, String)> = census()
        .observed
        .into_iter()
        .map(|row| (row.module, row.ns))
        .collect();
    for ruleset in RULESETS {
        let mut engine = production_engine(ruleset);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine
            .modules_loaded(HashMap::new(), &mut out, &mut fx)
            .expect("the production roster boots on an empty store");
        let undeclared: Vec<String> = engine
            .world
            .foreign_namespace_writes()
            .into_iter()
            .filter(|(module, ns, _)| !declared.contains(&(module.to_string(), ns.to_string())))
            .map(|(module, ns, owner)| {
                format!(
                    "{module} -> {ns} (owned by {})",
                    owner.as_deref().unwrap_or("nobody")
                )
            })
            .collect();
        assert!(
            undeclared.is_empty(),
            "[{ruleset}] boot performed cross-namespace module write(s) nobody declared: \
             {undeclared:?}. Add a row to {ALLOWLIST} saying how the write gets there, or \
             stop making it — WS03-A1's second commit turns this into a refusal."
        );
    }
}

/// **Every census row is legible.** A row names a real module, a real
/// namespace, and the route between them; a row without a route is a row
/// nobody can act on when the refusal drops the write it excuses.
#[test]
fn every_census_row_names_a_module_a_namespace_and_a_route() {
    let rows = census().observed;
    assert!(
        !rows.is_empty(),
        "{ALLOWLIST} is empty, but the WS03-A1 census run observed cross-namespace writes. \
         An empty file would claim the delegation set is empty."
    );
    let mut composed: BTreeSet<String> = BTreeSet::new();
    for ruleset in RULESETS {
        composed.extend(module_names(&production_engine(ruleset)));
    }
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for row in &rows {
        assert!(
            !row.reason.trim().is_empty(),
            "{ALLOWLIST}: {} -> {} carries no route",
            row.module,
            row.ns
        );
        assert_ne!(
            row.module, row.ns,
            "{ALLOWLIST}: {} -> {} is a module writing its OWN namespace, which is never a \
             cross-namespace write",
            row.module, row.ns
        );
        assert!(
            seen.insert((row.module.clone(), row.ns.clone())),
            "{ALLOWLIST}: {} -> {} appears twice; one row per (module, namespace) pair",
            row.module,
            row.ns
        );
        assert!(
            composed.contains(&row.module),
            "{ALLOWLIST}: {} is not a module any production composition builds",
            row.module
        );
        assert!(
            composed.contains(&row.ns),
            "{ALLOWLIST}: {} is not a namespace any production module owns",
            row.ns
        );
    }
}
