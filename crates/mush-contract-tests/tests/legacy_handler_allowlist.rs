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
//! server's staff commands, `server_modules_with`, worldgen, the NPC module, the
//! game's character sheet, the TacSim layer and the content module, with default
//! configuration.
//!
//! **16b cut R-b: there is exactly ONE composition.** This file used to build one
//! per `[rulesets].enabled` id and take the union, because rustmush ran one
//! character ruleset per server and `wod`/`shadowrun` collided on
//! `+roll`/`+sheet`/`+chargen`. Both rulesets are gone, `sheet_modules` is
//! infallible and composes the one game sheet unconditionally, and no config knob
//! changes which verbs exist — so the "union over every single-ruleset
//! composition" is the one composition, and [`RULESETS`] is gone with the loop it
//! drove.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use mush_core::{CommandCompatibility, Engine, GameModule, LookupErr, ModuleComposition};
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

/// How many `baseline` entries the pin carries — every command registered on
/// `main` at `b0e3b52`, **less the ones since re-categorized**. A baseline entry may
/// be **retired** with its verb, or **re-declared** under a real category once it
/// stops being the handler that was pinned (lower this number in the same commit);
/// one may never be **added**: a new handler is listed with a non-baseline category.
///
/// 159 → 158 with the E6b independent verify fold (D9): `+scene` was pinned as a
/// read with no switches and no arguments, and E6b gave it `/outcome`, which writes
/// a terminal sealed outcome and commits canon an escrow settles from. It is
/// `legacy-post-image` now, with its family and its reasons in the stanza. The
/// tripwire still fails on an **unlisted** verb, which is what it is for.
///
/// 158 -> 157 with the E7 cut 8 independent verify fold (B1): `@domain` was
/// pinned as a baseline verb and cut 8 gave it `/tick`, whose handler reaches
/// `Ctx::submit_authoritative(domain_pulse_arm, ..)` and arms the domain's
/// durable pulse cursor. It is `authoritative` now, with its reasons in the
/// stanza.
///
/// 151 -> 98 with 16b cut R-b (2026-09-16): the two character rulesets left the
/// composition, and 53 baseline verbs retired with them (the 49 pinned as one
/// composition only, plus `+cast`, `+heal`, `+resonance` and `+spec`, which both
/// registered and nothing else did). The one game sheet registers `+sheet`,
/// `@sheet`, `+chargen`, `+stat` and `+roll` under the same names.
// 98 -> 97 with cut L: @siteban/lift adds an operational admission workflow.
// 97 -> 0 with P3 (doc 28 §4 Finding 3 / §5): every remaining `baseline` entry
// is classified under its K8.5 category (`read-only`, `legacy-post-image` with
// `family`, `authoritative`, or `economy-carve-out`) and cross-checked against
// the class published by the production composition.
const PINNED_BASELINE: usize = 0;

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

/// The production module roster with default configuration, composed exactly as
/// `build_engine` composes it.
///
/// 16b cut R-b: no ruleset argument. `[rulesets].enabled` selected one of two
/// character rulesets and is read by nothing; the one game sheet
/// (`sheet_modules`) composes unconditionally and infallibly.
fn production_modules() -> Vec<Box<dyn GameModule>> {
    let config = mush_config::config::Config::default();
    // The worldgen module wants an LLM service; no provider is configured and the
    // service is disabled, which changes nothing about which verbs register.
    let llm = Arc::new(
        mush_llm::LlmService::builder()
            .global_enabled(false)
            .build(),
    );
    let onboarding = config_ext::onboarding_config(&config.onboarding);
    let game = config_ext::tacsim_economy_config(&config.tacsim)
        .expect("the default [tacsim.economy] converts");
    let (mut mods, inventory) = modules::server_modules_with_tacsim(
        config_ext::weather_config(&config.weather),
        config_ext::economy_config(&config.economy).expect("the default [economy] converts"),
        config_ext::atlas_config(&config.atlas, &config.worldgen)
            .expect("the default [atlas] converts"),
        &onboarding,
        &game,
        // E7r1 R4: this composition carries no `[tacsim.recovery]`, so the provider
        // is disabled — `@recovery` still registers, which is exactly what lets this
        // tripwire see it.
        modules::no_recovery(),
    );
    mods.push(modules::worldgen_module_with(
        Arc::clone(&llm),
        config_ext::worldgen_config(&config.worldgen),
    ));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(&onboarding, &game));
    // E7 cut 1: the TacSim layer is part of every production composition — it is a
    // layer switched by `[tacsim].enabled` (doc 10 §20) and it composes after the
    // character sheet. Default configuration means `enabled = false`, and the verbs
    // still register (they answer "not enabled on this server"), which is exactly
    // what makes the surface the tripwire watches independent of the operator's
    // switches.
    // Doc 18 cut IC8: composed as `boot::game_modules` composes it — the layer the
    // third quantity delegate, holding the inventory cell.
    mods.push(modules::tacsim_module_wired(
        config_ext::tacsim_config(&config.tacsim).expect("the default [tacsim] converts"),
        false,
        inventory,
    ));
    // S1: the content module last, as `build_engine` composes it. It registers no
    // verb; it is here so this is the production roster and not most of it.
    mods.push(
        modules::content_module(config.game.starter_area).expect("the content module composes"),
    );
    mods
}

/// `build_engine`'s own step: compose and freeze every registry before any world
/// record exists. The verbs are fixed here; no world and no boot are needed.
fn production_composition() -> ModuleComposition {
    let staff = Arc::new(StaffQueue::new());
    let mut registry = base_registry();
    register_server_commands(&mut registry, &staff)
        .expect("server command registration must not collide");
    Engine::compose_modules(registry, production_modules())
        .expect("the production composition registers without collision")
}

/// The primary verb of every command the production server registers with default
/// configuration (help topics are prose, not commands, and are not loaded).
fn production_commands() -> BTreeSet<String> {
    production_composition()
        .registry()
        .specs()
        .map(|spec| spec.name.to_string())
        .collect()
}

/// **The class the production composition actually publishes for each verb**
/// (independent verify fold, B8; owner decision F10(a)).
///
/// `legacy_handler_allowlist.rs` never mentioned `CommandCompatibility` at all, so a
/// stanza's declared `category` and the class the composed registry publishes could
/// drift silently — which is precisely how a pass that refreshed two stanzas left
/// `@wo`, the one verb whose lane changed, carrying cut 4's note and an unchecked
/// class. The composition is the same one `production_commands` builds; only the
/// projection differs.
fn published_classes() -> BTreeMap<String, CommandCompatibility> {
    let composition = production_composition();
    let registry = composition.registry();
    registry
        .specs()
        .filter_map(|spec| {
            registry
                .compatibility_of(&spec.name)
                .map(|class| (spec.name.to_string(), class))
        })
        .collect()
}

/// **Category/class drift recorded but not fixed** — `(verb, declared category,
/// published class)`.
///
/// **EMPTY, and it stays empty** (E7 residue pass 2, P0c). E7 residue pass 1's fold
/// added this cross-check and the first thing it did was find two verbs whose stanza
/// and registration had disagreed since before that pass; its own rules permitted one
/// edit to `legacy-handlers.toml`, so it recorded them here rather than re-categorising
/// them silently. Pass 2 fixed both, each on the side the tree makes true:
///
/// * **`@station`** declared `legacy-post-image` and published `Authoritative`. The
///   registration was right — `/handoff` and `/cancel` ride cut 2's authoritative
///   ingress and a `CommandSpec` takes the ceiling class of its switches — so the
///   **stanza** moved to `authoritative`, and its `family` key was dropped with it:
///   §14.4 puts `sim.station_fallback` on the list of workflow documents that
///   deliberately have **no** E7k lane, so naming it as a future typed family made the
///   migration plan work that is not planned.
/// * **`+nuyen`** declared `read-only` and published `LegacyPostImage`. Here the
///   **stanza** was right and the registration was wrong: `nuyen_cmd` is a pure render
///   and writes nothing, and it published a post-image class only because
///   `ShadowrunRuleset::register` classifies its whole module at once. It is now its
///   own K8.5 registration group, `ReadOnly` before its own `reg.command` with
///   `LegacyPostImage` restored after. Declaring it `legacy-post-image` instead would
///   have needed a fabricated future family for a write path that does not exist.
///
/// Neither retired a baseline stanza, so `PINNED_BASELINE` is unmoved at 157. An entry
/// added here is a defect that must be fixed by its owning cut; the assertion below
/// makes a stale exception impossible to leave behind.
const KNOWN_CLASS_DRIFT: &[(&str, &str, &str)] = &[];

/// The category word a published [`CommandCompatibility`] corresponds to.
fn category_of(class: CommandCompatibility) -> &'static str {
    match class {
        CommandCompatibility::ReadOnly => "read-only",
        CommandCompatibility::LegacyPostImage => "legacy-post-image",
        CommandCompatibility::Authoritative => "authoritative",
        CommandCompatibility::EconomyCarveOut => "economy-carve-out",
    }
}

/// **Every stanza's `category` equals the class the production composition publishes
/// for that verb** (B8; F10(a)) — the cross-check that did not exist.
///
/// `baseline` is exempt by construction: it is a provenance word, not a class, and
/// the verbs it covers publish whatever their pre-K8.5 registration published.
/// Every non-baseline stanza is a deliberate declaration and is checked against the
/// one production composition (16b cut R-b: there is no longer a per-ruleset
/// composition for a class to differ between).
#[test]
fn vf_b8a_every_stanzas_category_matches_the_class_the_composition_publishes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let list: Allowlist = toml::from_str(&text)
        .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
    let mut problems: Vec<String> = Vec::new();
    let mut seen_drift: BTreeSet<&str> = BTreeSet::new();
    {
        let published = published_classes();
        for handler in &list.handler {
            if handler.category == "baseline" {
                continue;
            }
            let Some(class) = published.get(&handler.name) else {
                continue;
            };
            let expected = category_of(*class);
            if expected == handler.category {
                continue;
            }
            if let Some(known) = KNOWN_CLASS_DRIFT.iter().find(|(name, declared, actual)| {
                *name == handler.name && *declared == handler.category && *actual == expected
            }) {
                seen_drift.insert(known.0);
                continue;
            }
            problems.push(format!(
                "{:?}: the stanza declares category {:?}, but the composition publishes \
                 {expected:?} ({class:?}). Either the registration moved lanes without the \
                 stanza, or the stanza claims a lane the verb does not ride. If this is \
                 deliberate and cannot be fixed in this commit, add it to KNOWN_CLASS_DRIFT \
                 with the reason.",
                handler.name, handler.category
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{}",
        problems.join(
            "
"
        )
    );
    // The recorded drift is a **ceiling**, not a floor: fixing one must shrink this
    // list, so a stale exception cannot outlive the defect it excuses.
    let recorded: BTreeSet<&str> = KNOWN_CLASS_DRIFT.iter().map(|(name, _, _)| *name).collect();
    assert_eq!(
        seen_drift, recorded,
        "KNOWN_CLASS_DRIFT names a verb whose category and published class now AGREE:          remove it from the list in the same commit that fixed it"
    );
}

/// **`@wo`'s note records the money leg** (B8's other half: the class ceiling was
/// right and the note was cut 4's, ending *"money stays an E4 Hold and never grows a
/// resource field"* while the handler had grown a durable `mush_economy::post`).
#[test]
fn vf_b8b_the_wo_note_records_the_money_leg() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let list: Allowlist = toml::from_str(&text)
        .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
    let wo = list
        .handler
        .iter()
        .find(|h| h.name == "@wo")
        .expect("the @wo stanza");
    assert_eq!(
        wo.category, "authoritative",
        "F10(a): the class does not move"
    );
    let note = wo.note.as_deref().unwrap_or_default();
    for phrase in [
        "industry.wo.hold/{order}/{n}",
        "industry.wo.settle/{order}/{n}",
        "ENGINE_OWNED_RESERVATIONS",
        "SWEEP_TAG",
        "carve-out lane",
        "attempt",
    ] {
        assert!(
            note.contains(phrase),
            "the @wo note must record {phrase:?}: {note}"
        );
    }
}

/// Every verb the production composition registers.
///
/// 16b cut R-b: this used to be the UNION over every single-ruleset composition.
/// There is one composition, so the union is it — the name is kept because the
/// allowlist's own language ("every command the production server registers") is
/// what the tripwire below is written against.
fn registered_commands() -> BTreeSet<String> {
    production_commands()
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

/// The pinned file's own consistency: the surface the tripwire watches is not
/// empty, and the game's own sheet is really in it — so a module that stops
/// composing cannot silently shrink what the allowlist is measured against.
///
/// 16b cut R-b: this used to walk `RULESETS` and check each single-ruleset
/// composition contributed to the union. With one composition the union claim is
/// a tautology, so what is left is the claim that made it worth having — a
/// composition that quietly lost a module would shrink `registered_commands`,
/// and every stanza for the verbs it had would then fail as "listed but nothing
/// registers it" with no hint why. These rows say why.
#[test]
fn the_production_composition_carries_the_surface_the_allowlist_watches() {
    let names = registered_commands();
    assert!(!names.is_empty(), "no commands registered");
    // The one game sheet's verbs (`pilot`), the TacSim layer's (`tacsim`), and a
    // built-in from each end of the roster — enough that a dropped module is a
    // named failure rather than a wall of unlisted stanzas.
    for verb in [
        // the game sheet
        "+sheet",
        "+chargen",
        "+stat",
        "+roll",
        // the TacSim layer
        "@tacsim",
        "@corridor",
        // M6's owned author workflow and private replay surface
        "@mind",
        "bench",
        // the NPC module, and worldgen
        "@agent",
        "@gen",
        // the economy's two title-minting layers
        "@market",
        "@property",
    ] {
        assert!(
            names.contains(verb),
            "{verb} is not registered: the production composition dropped a module"
        );
    }
}

/// DT.1b uses the ordinary command registry. Pin its prefix effects so a later
/// alias cannot silently turn a roster acknowledgement into another command.
#[test]
fn practice_detachment_commands_keep_their_declared_class_and_prefixes() {
    let composition = production_composition();
    let registry = composition.registry();
    for verb in ["+sortie", "wilco", "negative"] {
        let (spec, _) = registry
            .lookup(verb)
            .expect("detachment command registered");
        assert_eq!(spec.name.as_str(), verb);
        assert!(spec.aliases.is_empty(), "{verb} adds no command aliases");
        assert_eq!(
            registry.compatibility_of(verb),
            Some(CommandCompatibility::Authoritative),
            "{verb} shares the cockpit's command lane"
        );
    }
    for (prefix, expected) in [
        ("n", "negative"),
        ("ne", "negative"),
        ("neg", "negative"),
        ("nega", "negative"),
        ("negat", "negative"),
        ("negati", "negative"),
        ("negativ", "negative"),
        ("wi", "wilco"),
        ("wil", "wilco"),
        ("wilc", "wilco"),
    ] {
        let (spec, _) = registry
            .lookup(prefix)
            .expect("unique acknowledgement prefix");
        assert_eq!(spec.name.as_str(), expected, "prefix {prefix}");
    }
    // A direction exit still resolves before this registry. Without an exit,
    // n/ne now resolve to NEGATIVE; w remains ambiguous as it was before DT.1b.
    match registry.lookup("w") {
        Err(LookupErr::Ambiguous(names)) => {
            let names: BTreeSet<&str> = names.iter().map(|name| name.as_str()).collect();
            assert_eq!(
                names,
                BTreeSet::from(["WHO", "watch", "weaponstatus", "whisper", "wilco"])
            );
        }
        _ => panic!("w must remain an ambiguous command prefix"),
    }
    assert!(matches!(registry.lookup("+sort"), Err(LookupErr::NotFound)));
}
