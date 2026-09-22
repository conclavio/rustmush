//! WS03-S8/S14: module-state slot ownership, pinned.
//!
//! A typed state slot is declared by exactly one module (`Registrar::state`),
//! and the world records that module as its owner — by MODULE name, not by
//! crate, so a crate registering two modules is two owners. Since WS03-S14 the
//! module-facing surface REFUSES a foreign write: `Ctx::try_state_mut` returns
//! `Err(StateSlotDenied { requester, owner, .. })` and `Ctx::state_mut` returns
//! `None`. That flip is silent at every call site
//! (`if let Some(s) = ctx.state_mut::<T>()` simply becomes a no-op), which is
//! why WS03-S8 measured the baseline first and why this file keeps it from
//! growing quietly afterwards.
//!
//! Two claims, in the `legacy_handler_allowlist.rs` style — a checked-in
//! allowlist beside this crate's `Cargo.toml`, a scan that fails on an
//! undeclared hit, and a message naming the file to edit:
//!
//! 1. **Ownership resolves.** Every slot the production composition installs
//!    has exactly one owning module, and the slots whose owner DEPENDS on which
//!    ruleset composed are pinned by name ([`SHARED_RULESET_SLOTS`]) rather than
//!    averaged away. The composition is the one root
//!    both binaries boot — `mush_server::boot::game_modules` — reached the way
//!    `legacy_handler_allowlist.rs` reaches it (this crate has a `mush-server`
//!    dev-dependency and deliberately no `mushd` one: `mushd/src/boot.rs` is a
//!    delegator, and asserting against it would assert against a transcription).
//!
//! 2. **Cross-owner writes are baselined.** The scan reads every `.rs` file
//!    under `crates/` and resolves each
//!    `ctx.state_mut::<T>` / `World::module_state_mut::<T>` /
//!    `World::module_state_insert(..)` site against the owner map. A site whose
//!    slot belongs to a module of a DIFFERENT crate must carry a row in
//!    `state-slot-owners.toml` saying why; so must every
//!    `module_state_insert` site, which is the accessor that seeds a slot with
//!    no owner at all.
//!
//! # What the scan can and cannot see
//!
//! The scan is crate-level: it knows the crate a call site is in and the crate
//! that defines the slot type, and it treats same-crate access as in-family.
//! Module-level truth — a crate registering two modules, one of them writing
//! the other's slot — is only visible at RUNTIME, which is what the refusal
//! at `Ctx` is for and what the (now empty) `[[observed]]` rows recorded. The
//! two halves are deliberately different instruments: the scan cannot be
//! fooled by a code path no test exercises, and the runtime check cannot be
//! fooled by a crate boundary that is not a module boundary.
//!
//! `World::module_state_mut` and `World::module_state_insert` stay `pub`: they
//! are the unchecked engine accessors, carrying no module identity and doing no
//! check. What keeps them from being the way around slot ownership is this
//! file — a convention pinned by a scan, not an invariant enforced by the
//! types — and both accessors say so in their own docs.
//!
//! That matters more than it looks: `Ctx::w` is a `pub` field, so
//! `ctx.w.module_state_mut::<T>()` is one character longer than
//! `ctx.state_mut::<T>()` and skips the refusal entirely. The census found that
//! four crates' production code already takes exactly that route across a
//! module boundary (both rulesets and `mush-faction` into the economy and org
//! stores, `mush-sim` into `scenes`). WS03-S14's refusal therefore does not
//! bound the reach on its own; this scan is the other half of the enforcement,
//! not a nicety beside it, and it is why a `[[site]]` row may name the
//! unchecked accessors and may NOT name `Ctx::state_mut`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mush_core::module::StateSlotInfo;
use mush_core::{Engine, World};
use mush_server::boot;
use mush_server::staff::{StaffQueue, register_server_commands};
use mush_server::world_task::base_registry;
use serde::Deserialize;

/// The checked-in baseline, beside this crate's `Cargo.toml`.
const ALLOWLIST: &str = "state-slot-owners.toml";

/// 16b cut R-b: there is one production composition, the game's. The label is
/// what the messages print; the loops that once ranged over the rulesets stay,
/// so a second composition cannot arrive without widening this list.
const RULESETS: [&str; 1] = ["tacsim"];

/// The three accessors the scan knows, spelled as a row must spell them.
const ACCESSORS: [&str; 3] = [
    "Ctx::state_mut",
    "World::module_state_mut",
    "World::module_state_insert",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    /// Call sites the scan finds and the census excuses.
    #[serde(default)]
    site: Vec<Site>,
    /// Foreign writes the WS03-S8 census RUN actually observed (the workspace
    /// suite executed with the warn installed). Module-level, so it sees what
    /// the crate-level scan cannot.
    #[serde(default)]
    observed: Vec<Observed>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Site {
    /// Workspace-relative path, forward slashes.
    path: String,
    /// The accessor, exactly one of [`ACCESSORS`].
    accessor: String,
    /// The slot as the source writes it (the turbofish type, or the leading
    /// identifier of a `module_state_insert` argument).
    slot: String,
    /// The module that owns the slot, or `""` when no production composition
    /// declares it.
    owner: String,
    /// Why this reach is allowed to exist.
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observed {
    /// The writing module, as `GameModule::name` reports it.
    module: String,
    /// The slot's `std::any::type_name`.
    slot: String,
    /// What the census run was and what the pair means.
    note: String,
}

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

fn baseline() -> Baseline {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// The production composition
// ---------------------------------------------------------------------------

/// Every state slot the production server installs with default configuration,
/// built through the ONE composition root both halves boot.
fn production_slots(_composition: &str) -> Vec<StateSlotInfo> {
    let config = mush_config::config::Config::default();
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
    // Installing the composition is what records the owners.
    let engine = Engine::from_composition(World::new_minimal(), composition);
    engine.world.module_state_slots()
}

/// `type name -> the module(s) that declare it`, unioned over every production
/// composition.
///
/// One composition can only ever give a slot ONE owner —
/// `Engine::compose_modules` refuses a second module declaring the same
/// `TypeId`. The union can still hold two, because a server runs exactly one
/// character ruleset and the alternatives are never composed together; see
/// [`SHARED_RULESET_SLOTS`].
fn owner_map() -> BTreeMap<&'static str, BTreeSet<String>> {
    let mut owners: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for ruleset in RULESETS {
        for slot in production_slots(ruleset) {
            let Some(owner) = slot.owner else { continue };
            owners
                .entry(slot.type_name)
                .or_default()
                .insert(owner.to_string());
        }
    }
    owners
}

/// Slot types declared by MORE THAN ONE module across the compositions —
/// pinned, because the only reason that is legal is that the owners are
/// alternative single-ruleset compositions that never run together.
///
/// Both are `mush-ruleset`'s vitals slots — the sheet-schema fingerprints and
/// the vitals revision counters. They live in the shared `mush-ruleset` crate
/// and are declared by whichever character ruleset composes, so `wod` owns them
/// on a WoD server and `shadowrun` owns them on a Shadowrun one.
///
/// A type joining this list because two modules of the SAME composition declare
/// it cannot happen (the compose step refuses it); a type joining it for any
/// other reason is a slot whose owner depends on configuration, and WS03-S14
/// has to know that before it refuses anything.
///
/// 16b cut R-b: one composition, so nothing is shared any more; the pin stays
/// empty so a slot with two owners is a reviewed decision again.
const SHARED_RULESET_SLOTS: [&str; 0] = [];

/// The last segment of a `std::any::type_name` path, generics stripped —
/// `mush_npc::state::NpcState` becomes `NpcState`, which is how source writes
/// it.
fn short_name(type_name: &str) -> &str {
    let head = type_name.split('<').next().unwrap_or(type_name);
    head.rsplit("::").next().unwrap_or(head).trim()
}

/// `short type name -> every (owner, full type name) pair with that short
/// name`, packed one per string so the scan can compare both halves.
fn short_owner_map(
    owners: &BTreeMap<&'static str, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut short: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (type_name, module) in owners {
        for owner in module {
            short
                .entry(short_name(type_name).to_string())
                .or_default()
                .insert(format!("{owner}\u{1}{type_name}"));
        }
    }
    short
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// One call site the scan found.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Hit {
    path: String,
    accessor: &'static str,
    slot: String,
}

/// Blank line comments so a doc comment naming an accessor is not a hit.
///
/// Line-based, not a lexer: a `"…//…"` string literal blanks the rest of its
/// own line, which can only ever hide a hit that shares a line with a string
/// containing `//`. Nothing in the tree does that, and the failure mode is a
/// missing hit rather than a fabricated one.
fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => line[..at].to_string(),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The turbofish type at `rest` (`::<Foo>(` → `Foo`), if there is one.
fn turbofish(rest: &str) -> Option<String> {
    let rest = rest.strip_prefix("::<")?;
    let end = rest.find('>')?;
    Some(short_name(&rest[..end]).to_string())
}

/// The leading identifier of a call argument (`(NpcState::default())` →
/// `NpcState`).
fn first_argument(rest: &str) -> Option<String> {
    let rest = rest.strip_prefix('(')?;
    let arg: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
        .collect();
    let head = arg.split("::").next().unwrap_or(&arg).trim();
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

/// Every accessor site in one file.
fn hits_in(rel_path: &str, source: &str) -> Vec<Hit> {
    let code = strip_line_comments(source);
    let mut hits = Vec::new();
    for (needle, accessor) in [
        ("module_state_insert", ACCESSORS[2]),
        ("module_state_mut", ACCESSORS[1]),
        ("state_mut", ACCESSORS[0]),
    ] {
        let mut from = 0usize;
        while let Some(at) = code[from..].find(needle) {
            let start = from + at;
            from = start + needle.len();
            // `module_state_mut` also ends in `state_mut`; each needle claims
            // its own sites, longest first.
            if accessor == ACCESSORS[0] && code[..start].ends_with("module_") {
                continue;
            }
            // A definition is not a call site.
            if code[..start].ends_with("fn ") {
                continue;
            }
            let rest = &code[from..];
            let slot = if accessor == ACCESSORS[2] {
                // A seeded slot's type is usually inferred from the argument.
                turbofish(rest).or_else(|| first_argument(rest))
            } else {
                turbofish(rest)
            };
            let Some(slot) = slot else { continue };
            hits.push(Hit {
                path: rel_path.to_string(),
                accessor,
                slot,
            });
        }
    }
    hits.sort();
    hits.dedup();
    hits
}

/// Every `.rs` file under `crates/`, workspace-relative with forward slashes.
fn workspace_sources(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if entry.file_name() == "target" {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&root.join("crates"), &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let rel = path
                .strip_prefix(root)
                .expect("walked from the root")
                .to_string_lossy()
                .replace('\\', "/");
            (rel, path)
        })
        .collect()
}

/// The crate a workspace-relative path belongs to, as a type path writes it
/// (`crates/mush-npc/src/x.rs` → `mush_npc`).
fn crate_of(rel_path: &str) -> Option<String> {
    rel_path
        .strip_prefix("crates/")
        .and_then(|rest| rest.split('/').next())
        .map(|name| name.replace('-', "_"))
}

/// The module name a crate is the home of, by the tree's own convention:
/// `mush_tactics` is where the `tactics` module lives, `ruleset_wod` is where
/// `wod` does.
///
/// This is what lets the crate-level scan see that `mush-tactics` writing
/// `ContactLedgerState` — a type that lives in `mush-core` but is DECLARED by
/// the `tactics` module — is a module writing its own slot. Without it the scan
/// would demand a row for every slot whose type is defined in a crate other
/// than the one whose module owns it.
fn crate_family(crate_name: &str) -> &str {
    crate_name.strip_prefix("mush_").unwrap_or(crate_name)
}

/// Every module that owns a slot with this short name (empty when no
/// production composition declares one).
fn resolved_owners(short: &BTreeMap<String, BTreeSet<String>>, slot: &str) -> BTreeSet<String> {
    short
        .get(slot)
        .map(|candidates| {
            candidates
                .iter()
                .filter_map(|c| c.split_once('\u{1}').map(|(owner, _)| owner.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// A row's `owner` must be the module the composition says owns that slot —
/// or `""` when nothing declares it, which is what an unowned seeded slot is.
/// A row that names the wrong owner is a row nobody re-read.
fn row_owner_is_right(row: &Site, resolved: &BTreeSet<String>) -> bool {
    if resolved.is_empty() {
        row.owner.is_empty()
    } else {
        resolved.contains(&row.owner)
    }
}

fn owner_mismatch(row: &Site, resolved: &BTreeSet<String>) -> String {
    let expected: Vec<&str> = resolved.iter().map(String::as_str).collect();
    format!(
        "{ALLOWLIST}: the row for {} / {} declares owner {:?}, but the production composition \
         says {:?}. Fix the row.",
        row.path,
        row.slot,
        row.owner,
        if expected.is_empty() {
            vec![""]
        } else {
            expected
        }
    )
}

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// **Every declared slot resolves to exactly one owner.**
///
/// A slot with no owner in a freshly composed world would mean a state slot
/// arrived without a `Registrar::state` declaration, which is the case WS03-S14
/// must leave open to all — it must not be possible to reach it by accident at
/// composition time, only by seeding one deliberately from `on_world_loaded`.
#[test]
fn every_production_state_slot_has_exactly_one_owner() {
    for ruleset in RULESETS {
        let slots = production_slots(ruleset);
        assert!(
            slots.len() >= 20,
            "[{ruleset}] only {} slots — the composition did not build",
            slots.len()
        );
        let unowned: Vec<&'static str> = slots
            .iter()
            .filter(|s| s.owner.is_none())
            .map(|s| s.type_name)
            .collect();
        assert!(
            unowned.is_empty(),
            "[{ruleset}] state slots installed with no declaring module: {unowned:?}. \
             A composed slot comes from Registrar::state and carries its module; a slot \
             seeded through World::module_state_insert has no owner and is warned at boot."
        );
    }
    let owners = owner_map();
    assert!(
        owners.len() >= 20,
        "the owner map is suspiciously small: {}",
        owners.len()
    );
    // One composition, one owner per slot — the compose step refuses a second
    // declaration. Across compositions a slot may still have two owners, and
    // exactly which slots those are is pinned rather than averaged away.
    let shared: BTreeMap<&str, &BTreeSet<String>> = owners
        .iter()
        .filter(|(_, module)| module.len() > 1)
        .map(|(type_name, module)| (*type_name, module))
        .collect();
    let expected: BTreeSet<&str> = SHARED_RULESET_SLOTS.into_iter().collect();
    let found: BTreeSet<&str> = shared.keys().copied().collect();
    assert_eq!(
        found, expected,
        "a slot's owner now depends on which ruleset composes (or one stopped doing so): \
         {shared:?}. That is legal only while the owners are alternative single-ruleset \
         compositions — update SHARED_RULESET_SLOTS in the same commit, and tell WS03-S14."
    );
}

/// **Every cross-owner state-mutation site is baselined**, and every baselined
/// row still names a real one.
///
/// The census is the point: WS03-S14 flips `Ctx::state_mut` to deny a foreign
/// write, and a denial is invisible at the call site, so the set of sites that
/// would start no-opping has to be written down first.
#[test]
fn every_cross_owner_state_site_is_baselined() {
    let root = repo_root();
    let owners = owner_map();
    let short = short_owner_map(&owners);
    let list = baseline();

    for site in &list.site {
        assert!(
            ACCESSORS.contains(&site.accessor.as_str()),
            "{ALLOWLIST}: unknown accessor {:?} (one of {ACCESSORS:?})",
            site.accessor
        );
        assert!(
            !site.reason.trim().is_empty(),
            "{ALLOWLIST}: {} / {} carries no reason",
            site.path,
            site.slot
        );
    }
    let declared: BTreeMap<(String, String, String), &Site> = list
        .site
        .iter()
        .map(|s| ((s.path.clone(), s.accessor.clone(), s.slot.clone()), s))
        .collect();

    let mut needed: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut problems: Vec<String> = Vec::new();
    for (rel, path) in workspace_sources(&root) {
        // The accessors are DEFINED here; mush-core is not a caller.
        if rel.starts_with("crates/mush-core/src/") {
            continue;
        }
        let Some(caller_crate) = crate_of(&rel) else {
            continue;
        };
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        for hit in hits_in(&rel, &source) {
            let key = (hit.path.clone(), hit.accessor.to_string(), hit.slot.clone());
            // Seeding a slot is the accessor that can create an UNOWNED slot,
            // so every site of it is declared, resolvable or not.
            if hit.accessor == ACCESSORS[2] {
                needed.insert(key.clone());
                match declared.get(&key) {
                    None => problems.push(format!(
                        "{}: {} on {:?} is not in {ALLOWLIST}. Seeding a slot directly records \
                         no owner, so every site of it is declared with its reason.",
                        hit.path, hit.accessor, hit.slot
                    )),
                    Some(row) => {
                        let resolved = resolved_owners(&short, &hit.slot);
                        if !row_owner_is_right(row, &resolved) {
                            problems.push(owner_mismatch(row, &resolved));
                        }
                    }
                }
                continue;
            }
            let Some(candidates) = short.get(&hit.slot) else {
                // Not a slot any production composition declares (a test-local
                // slot, or a generic parameter): unowned, open to all.
                continue;
            };
            // A short name can resolve to more than one (owner, type) pair —
            // the alternative rulesets' shared slot. The site is this crate's
            // own if ANY of them is; only when none is does it need a row.
            let mut foreign: Vec<String> = Vec::new();
            let mut ours = false;
            for candidate in candidates {
                let (owner, type_name) = candidate
                    .split_once('\u{1}')
                    .expect("the owner map packs owner and type name");
                let defining_crate = type_name.split("::").next().unwrap_or(type_name);
                if defining_crate == caller_crate || owner == crate_family(&caller_crate) {
                    ours = true;
                } else {
                    foreign.push(format!("{type_name} (module {owner})"));
                }
            }
            if ours || foreign.is_empty() {
                continue;
            }
            needed.insert(key.clone());
            match declared.get(&key) {
                None => problems.push(format!(
                    "{}: {} reaches {:?}, owned outside {caller_crate} — {}. Add a row to \
                     {ALLOWLIST} saying why this crate writes another module's slot, or stop \
                     writing it. WS03-S14 turns this into a refusal.",
                    hit.path,
                    hit.accessor,
                    hit.slot,
                    foreign.join(", ")
                )),
                Some(row) => {
                    let resolved = resolved_owners(&short, &hit.slot);
                    if !row_owner_is_right(row, &resolved) {
                        problems.push(owner_mismatch(row, &resolved));
                    }
                }
            }
        }
    }

    // A row that no longer names a real site is a stale excuse.
    for key in declared.keys() {
        if !needed.contains(key) {
            problems.push(format!(
                "{ALLOWLIST}: the row for {} / {} / {} names no site the scan finds any more — \
                 delete it in the commit that removed the reach.",
                key.0, key.1, key.2
            ));
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// **The census is spent.** Each `[[observed]]` row was a foreign write the
/// workspace suite actually produced while WS03-S8's warn still applied it.
/// WS03-S14 refuses those writes, so the list must now be EMPTY: a pair that
/// still appears is a write that no longer lands, which is a defect to fix or
/// a seam to add on the owning module — never a row to keep.
#[test]
fn the_observed_census_is_empty_now_that_the_refusal_is_live() {
    let list = baseline();
    for observed in &list.observed {
        assert!(
            !observed.module.trim().is_empty() && !observed.slot.trim().is_empty(),
            "{ALLOWLIST}: an [[observed]] row must name the writing module and the slot"
        );
        assert!(
            !observed.note.trim().is_empty(),
            "{ALLOWLIST}: the observed pair {} / {} carries no note",
            observed.module,
            observed.slot
        );
    }
    let pairs: Vec<String> = list
        .observed
        .iter()
        .map(|o| format!("{} -> {}", o.module, o.slot))
        .collect();
    assert!(
        pairs.is_empty(),
        "{ALLOWLIST}: {} [[observed]] foreign write(s) are now REFUSED at Ctx::try_state_mut, \
         not excused: {pairs:?}. Give the owning module a seam the caller can use, or stop the \
         write — the row cannot make it land again.",
        pairs.len()
    );
}

/// **No row can excuse a `Ctx` reach.** `Ctx::state_mut` / `Ctx::try_state_mut`
/// REFUSE a foreign module's write since WS03-S14, so a `[[site]]` row naming
/// that accessor across an owner boundary would be an excuse for a write that
/// does not happen — a row nobody can act on, describing a silent no-op.
///
/// The scan is crate-level and the refusal is module-level, so the two halves
/// answer different questions (a crate registering two modules is one crate and
/// two owners). This is where they meet: whatever the scan finds through `Ctx`
/// must be fixed at the call site, and only the unchecked engine accessors —
/// `World::module_state_mut` / `World::module_state_insert`, which carry no
/// module identity and cannot check — remain baselineable.
#[test]
fn no_baselined_site_reaches_through_the_checked_accessor() {
    let list = baseline();
    let through_ctx: Vec<String> = list
        .site
        .iter()
        .filter(|s| s.accessor == ACCESSORS[0])
        .map(|s| format!("{} / {}", s.path, s.slot))
        .collect();
    assert!(
        through_ctx.is_empty(),
        "{ALLOWLIST}: {ctx} refuses a foreign write since WS03-S14, so these rows excuse a \
         no-op: {through_ctx:?}. Fix the call site — route it through the owning module, or \
         through the unchecked engine accessor with a reason that says why.",
        ctx = ACCESSORS[0],
    );
}
