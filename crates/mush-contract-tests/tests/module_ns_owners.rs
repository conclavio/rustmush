//! WS03-A1: cross-namespace module writes, declared and policed.
//!
//! A module's [`GameModule::name`] **is** its persistence namespace, so
//! `op.ns != ctx.module` at `Ctx::persist` is one module writing documents
//! another module owns. Until WS03-A1 that was unverified by construction —
//! the doc comment said so in as many words — which meant the delegation set
//! was not merely unpoliced but *unknown*. The census commit measured it;
//! this one refuses what nobody declared.
//!
//! In the `state_slot_owners.rs` / `legacy_handler_allowlist.rs` style: a
//! checked-in file beside this crate's `Cargo.toml`, a scan that fails on an
//! undeclared hit, and a message that names the file to edit. Three claims:
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
//! 2. **The declared set is pinned.** What makes a cross-namespace write land
//!    is `Registrar::persist_delegation` in the declaring module, not a row
//!    here; this file is what stops the set growing (or shrinking) unread. A
//!    new delegation without a row fails, and so does a row no module declares
//!    any more.
//!
//! 3. **Nothing is refused at boot.** Booting each production roster must
//!    produce an empty `World::refused_namespace_writes`: a refusal is a write
//!    that did not land, which is a defect to fix or a delegation to declare —
//!    never something to leave running.
//!
//! # What this can and cannot see
//!
//! Booting a roster exercises `on_world_loaded` and nothing else, so claim 3
//! is a floor. The pairs that matter most are the ones only play reaches (the
//! jobs API, called under a producing module's identity), and those were
//! measured once by running the crate test suites with the census counter
//! installed. That is the honest instrument: `Ctx::persist`'s namespace is a
//! runtime value, not a syntactic one, so no scan of the source can enumerate
//! it — which is exactly why claim 2 pins the declarations instead.
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

/// The checked-in declaration set, beside this crate's `Cargo.toml`.
const ALLOWLIST: &str = "module-ns-owners.toml";

/// Every ruleset a production server can enable, plus the ruleset-less
/// composition — `[rulesets].enabled` is the only knob that changes which
/// modules compose.
const RULESETS: [&str; 3] = ["", "wod", "shadowrun"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Declared {
    /// The cross-namespace write delegations the composition declares.
    #[serde(default)]
    delegation: Vec<Delegation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Delegation {
    /// The module whose identity is on the write (`ctx.module`).
    writer: String,
    /// The namespace the write lands in (`op.ns`).
    ns: String,
    /// Which of the two modules declared the pair.
    declared_by: String,
    /// How a write gets from that module into that namespace.
    reason: String,
}

fn declared() -> Declared {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()))
}

/// A production server's engine with `[rulesets].enabled = [ruleset]` (or
/// nothing enabled, for `""`), built through the ONE composition root both
/// halves boot.
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

/// One declared pair, as both halves spell it.
type Row = (String, String, String);

/// Every delegation the compositions declare, unioned over the rulesets — a
/// ruleset only declares its pairs on the server that composes it.
fn composed_delegations() -> BTreeSet<Row> {
    let mut rows = BTreeSet::new();
    for ruleset in RULESETS {
        for d in production_engine(ruleset).registry.persist_delegations() {
            rows.insert((
                d.writer.to_string(),
                d.ns.to_string(),
                d.declared_by.to_string(),
            ));
        }
    }
    rows
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
             rule), but the refusal at Ctx::persist has to know about it — say so here in the \
             same commit."
        );
    }
}

/// **The declared set is exactly the pinned set.** A delegation is what lets a
/// cross-namespace write land, so one arriving (or leaving) unread is the
/// thing this file exists to prevent.
#[test]
fn every_declared_delegation_is_pinned_and_every_pinned_row_is_declared() {
    let rows = declared().delegation;
    assert!(
        !rows.is_empty(),
        "{ALLOWLIST} declares nothing, but the production composition delegates. An empty \
         file would claim the delegation set is empty."
    );
    let mut pinned: BTreeSet<Row> = BTreeSet::new();
    for row in &rows {
        assert!(
            !row.reason.trim().is_empty(),
            "{ALLOWLIST}: {} -> {} carries no route",
            row.writer,
            row.ns
        );
        assert_ne!(
            row.writer, row.ns,
            "{ALLOWLIST}: {} -> {} is a module writing its OWN namespace, which is never a \
             cross-namespace write",
            row.writer, row.ns
        );
        assert!(
            row.declared_by == row.writer || row.declared_by == row.ns,
            "{ALLOWLIST}: {} -> {} says it was declared by {}, which is neither the writer nor \
             the namespace's owner — Registrar::persist_delegation would have refused that",
            row.writer,
            row.ns,
            row.declared_by
        );
        assert!(
            pinned.insert((row.writer.clone(), row.ns.clone(), row.declared_by.clone())),
            "{ALLOWLIST}: {} -> {} appears twice; one row per (writer, namespace) pair",
            row.writer,
            row.ns
        );
    }
    let composed = composed_delegations();
    let missing: Vec<&Row> = composed.difference(&pinned).collect();
    assert!(
        missing.is_empty(),
        "the production composition declares delegation(s) {ALLOWLIST} does not pin: \
         {missing:?}. Add a row with the route, so a new cross-namespace write is reviewed \
         rather than noticed."
    );
    let stale: Vec<&Row> = pinned.difference(&composed).collect();
    assert!(
        stale.is_empty(),
        "{ALLOWLIST} pins delegation(s) no module declares any more: {stale:?}. Delete the row \
         in the commit that removed the Registrar::persist_delegation call."
    );
}

/// **Nothing is refused at boot.**
///
/// `on_world_loaded` runs with the same module identity a command does, so a
/// module reaching into another module's namespace at boot is the same
/// cross-namespace write and gets no exemption. Today no module makes one, and
/// a refusal here would be a boot write that silently stopped landing.
#[test]
fn booting_a_production_roster_refuses_no_module_write() {
    for ruleset in RULESETS {
        let mut engine = production_engine(ruleset);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine
            .modules_loaded(HashMap::new(), &mut out, &mut fx)
            .expect("the production roster boots on an empty store");
        let refused: Vec<String> = engine
            .world
            .refused_namespace_writes()
            .into_iter()
            .map(|(module, ns, owner)| {
                format!(
                    "{module} -> {ns} (owned by {})",
                    owner.as_deref().unwrap_or("nobody")
                )
            })
            .collect();
        assert!(
            refused.is_empty(),
            "[{ruleset}] boot made cross-namespace module write(s) that did NOT land: \
             {refused:?}. Declare the pair with Registrar::persist_delegation and add its row \
             to {ALLOWLIST}, or stop making the write."
        );
    }
}
