//! **The panic-shaped restriction lints** — a workspace policy gate (WS01-S12).
//!
//! WS01-S11 budgets the production panic surface by counting it. This is the
//! other half: where a crate's count for one of clippy's panic-shaped
//! restriction lints is already **zero**, the crate root denies that lint, and
//! the compiler holds it at zero. The budget is a ceiling that a reviewed diff
//! can raise; a deny is a floor that cannot be crossed at all.
//!
//! ## What was measured, and how
//!
//! Six candidates, over the 43-crate tree at this commit, with clippy 0.1.98:
//!
//! ```text
//! cargo clippy --workspace --all-targets --message-format=json -- \
//!   -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing \
//!   -W clippy::panic -W clippy::unreachable -W clippy::panic_in_result_fn
//! ```
//!
//! 6,678 warnings at 4,316 distinct source locations, every one of which would
//! be a hard error under `.github/workflows/ci.yml`'s `cargo clippy --workspace
//! --all-targets -- -D warnings`. That total is **not** the number a per-crate
//! enable is sized against, and the gap is the trap this file exists to keep
//! written down:
//!
//! | lint | all targets | `crates/*/src` only |
//! |---|---|---|
//! | `unwrap_used` | 1240 | **27** |
//! | `expect_used` | 2392 | **1206** |
//! | `indexing_slicing` | 454 | **324** |
//! | `panic` | 183 | **8** |
//! | `unreachable` | 36 | **28** |
//! | `panic_in_result_fn` | 11 | **3** |
//!
//! The right-hand column is what a crate-root `#![deny(...)]` polices: the lib
//! and the binaries. `crates/*/tests/*.rs` are separate crates and are not
//! covered by it — which matches WS01-S11's scope exactly, since the budget
//! counts `src/` and excludes `tests/`, `benches/` and `build.rs`. The
//! left-hand column is four times larger almost entirely because the root
//! `clippy.toml`'s `allow-*-in-tests` keys exempt `#[cfg(test)]` modules and
//! `#[test]` bodies but **not** the free helper functions in an integration
//! test file, which are ordinary functions.
//!
//! ## Why a crate-root attribute and not a `[lints]` table
//!
//! The step called for `[lints]` in each crate's manifest. Cargo 1.98 refuses
//! it: all 43 manifests inherit the workspace policy with `[lints] workspace =
//! true`, and adding a lint beside that is
//! `cannot override 'workspace.lints' in 'lints'` — the manifest route costs a
//! copy of `unsafe_code`/`dbg_macro`/`todo` in every crate that takes it, and
//! the copies drift the day the workspace set changes. A crate-root
//! `#![deny(...)]` keeps the workspace policy single-sourced, and it is the
//! shape `mush-server`'s `world_task.rs` already uses for its module-level
//! `#![deny(clippy::unreachable)]` (WS01-S5).
//!
//! ## Two clippy invocations, two platforms
//!
//! `ci.yml` clippies twice — the workspace with default features (on
//! ubuntu-latest **and** windows-latest), and `mush-net-web`/`mush-gateway`/
//! `mushd` again with the `tls` features. The `tls` run and a third run over
//! `mush-housing/debug-checks`, `mush-server/fault-injection`,
//! `mush-sim/testkit`, `mush-sim/debug-checks` and `mush-proto/ts` produce the
//! same `src/` counts as the default-feature run, so no feature toggle moves a
//! crate off this table. The measurement itself ran on Windows; the 40
//! `cfg(unix)`/`cfg(not(windows))` items under `crates/*/src` that Linux
//! compiles and Windows does not were read by hand, and the six that contain a
//! panic-shaped pattern are all inside `#[cfg(test)] mod tests` blocks, which
//! the root `clippy.toml` exempts.
//!
//! ## What is NOT enabled, and its number
//!
//! `MEASURED` below is the whole record: every crate, every candidate, and the
//! count that kept the lint off where it is off. The big two are off nearly
//! everywhere and stay advisory — `expect_used` is clean in 8 crates of 43 and
//! `indexing_slicing` in 14 — and `mush-store` is the one crate that pays for
//! all six, so it denies none of them. Nothing goes into
//! `[workspace.lints.clippy]`: with `-D warnings` in CI there is no such thing
//! as a workspace-wide `warn`, and none of the six is at zero everywhere.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The six candidates, in the order `MEASURED`'s counts are written.
const LINTS: [&str; 6] = [
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::indexing_slicing",
    "clippy::panic",
    "clippy::unreachable",
    "clippy::panic_in_result_fn",
];

/// Every workspace crate and its production (`src/`) count for each candidate,
/// measured with the command in this file's header.
///
/// A zero is a promise: that crate's roots deny the lint, and this gate fails
/// if one of them stops. A non-zero is the advisory record — the lint is off
/// there, and this is what turning it on would cost. Changing a count is how
/// you enable or retire a lint, and it is a reviewed diff either way.
/// Workspace crates outside the engine, exempt from this gate: their production
/// dependencies contain no engine, store or gateway, and they touch the game
/// only as a player would. `mush-bot` (the bot laboratory) talks to the server
/// over SSH; an unwrap in it ends a bot, never a world. The owner's call,
/// 2026-09-10, at S8's gate.
const OUT_OF_ENGINE: [&str; 1] = ["mush-bot"];

const MEASURED: [(&str, [u32; 6]); 43] = [
    ("mush-ansi", [0, 0, 13, 0, 0, 0]),
    // S1: born with all six denied (14-spatial-implementation-plan §3.1).
    ("mush-atlas", [0, 0, 0, 0, 0, 0]),
    ("mush-campaign", [0, 0, 4, 0, 0, 0]),
    ("mush-canon", [0, 7, 0, 0, 0, 0]),
    ("mush-comsys", [0, 13, 0, 0, 1, 0]),
    ("mush-config", [0, 4, 17, 0, 0, 0]),
    // S1: born with all six denied.
    ("mush-content", [0, 0, 0, 0, 0, 0]),
    ("mush-contract-tests", [0, 311, 4, 0, 3, 0]),
    ("mush-core", [0, 117, 94, 0, 4, 1]),
    ("mush-dice", [0, 0, 6, 0, 0, 0]),
    ("mush-economy", [0, 48, 22, 1, 1, 0]),
    ("mush-faction", [0, 36, 0, 0, 0, 0]),
    ("mush-gateway", [0, 3, 0, 0, 2, 0]),
    ("mush-housing", [0, 4, 2, 0, 0, 0]),
    ("mush-industry", [0, 4, 16, 0, 0, 0]),
    ("mush-inventory", [0, 5, 1, 0, 0, 0]),
    ("mush-ipc", [0, 0, 5, 0, 0, 0]),
    ("mush-jobs", [0, 2, 0, 0, 0, 0]),
    ("mush-llm", [0, 38, 1, 0, 0, 0]),
    ("mush-mail", [0, 10, 2, 0, 0, 0]),
    ("mush-market", [0, 2, 0, 0, 0, 0]),
    ("mush-net-discord", [0, 19, 1, 0, 0, 0]),
    ("mush-net-ssh", [0, 13, 28, 0, 1, 0]),
    ("mush-net-web", [0, 2, 1, 0, 0, 0]),
    ("mush-npc", [0, 2, 3, 0, 0, 0]),
    ("mush-org", [0, 27, 0, 0, 0, 0]),
    ("mush-party", [0, 0, 0, 0, 0, 0]),
    ("mush-privileges", [0, 4, 0, 0, 0, 0]),
    ("mush-propagation", [0, 2, 4, 0, 0, 0]),
    ("mush-proto", [0, 3, 0, 0, 0, 0]),
    ("mush-ruleset", [0, 3, 7, 0, 0, 0]),
    ("mush-scenes", [0, 12, 7, 0, 0, 0]),
    ("mush-script", [0, 1, 0, 0, 0, 0]),
    ("mush-server", [0, 29, 6, 1, 11, 0]),
    ("mush-session", [0, 0, 0, 0, 0, 0]),
    // 16b cut T (T.1, D-16-20): re-measured with this file's own header command
    // at the commit that retired the combat lane. `expect_used` 36 -> 35 is this
    // cut's (the deleted `combat.rs` and the deleted `tactical_intent` provider);
    // `unwrap_used` 6 -> 3, `expect_used` 42 -> 36 and `indexing_slicing` 7 -> 5
    // were drift the row had already accumulated over the earlier 16b cuts and
    // are folded in here rather than left wrong. No count crossed zero, so the
    // per-lint tally below does not move.
    ("mush-sim", [3, 35, 5, 0, 0, 0]),
    ("mush-store", [18, 283, 15, 1, 1, 2]),
    ("mush-tactics", [3, 49, 1, 0, 2, 0]),
    ("mush-testkit", [0, 5, 0, 1, 0, 0]),
    ("mush-weather", [0, 11, 11, 0, 0, 0]),
    ("mush-worldgen", [0, 0, 2, 0, 0, 0]),
    ("mushd", [0, 1, 0, 1, 0, 0]),
    // 16b cut R-a (R2): the composition root moved here from mush-server with its
    // three built-in-id `expect`s (housing and market title issuers, the capability
    // vocabulary); mush-server lost those three and the boot-loader one, which was
    // rewritten as a refusal.
    ("tacsim", [0, 3, 2, 0, 0, 0]),
];

/// The attribute heads that turn a lint **on** at a crate root.
const DENY_HEADS: [&str; 2] = ["#![deny(", "#![forbid("];

/// The attribute heads that turn one back **off** anywhere under `src/`. A
/// `warn` counts: it survives `cargo clippy` locally, and only CI's
/// `-D warnings` promotes it back to the error the deny meant.
const SILENCER_HEADS: [&str; 8] = [
    "#![allow(",
    "#[allow(",
    "#![expect(",
    "#[expect(",
    "#![warn(",
    "#[warn(",
    "#![cfg_attr(",
    "#[cfg_attr(",
];

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Reading the tree
// ---------------------------------------------------------------------------

/// The parenthesised group an attribute at `start` opens, or `None` when the
/// parentheses do not close.
fn attribute_group(text: &str, start: usize) -> Option<&str> {
    let open = start + text.get(start..)?.find('(')?;
    let mut depth = 0usize;
    for (offset, ch) in text.get(open..)?.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(open + 1..open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every `clippy::<lint>` path named inside an attribute group.
fn clippy_lints_in(group: &str) -> BTreeSet<String> {
    group
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == ':'))
        .filter(|piece| piece.starts_with("clippy::") && piece.len() > "clippy::".len())
        .map(str::to_string)
        .collect()
}

/// Every `clippy::<lint>` named by an attribute in `text` whose head is one of
/// `heads`.
///
/// A head must begin the line (after indentation), which is what keeps a `//!`
/// doc comment quoting an attribute — and every `.expect("...")` method call in
/// the file — out of the result.
fn lints_named_by(text: &str, heads: &[&str]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if heads.iter().any(|head| trimmed.starts_with(head))
            && let Some(group) = attribute_group(text, offset + indent)
        {
            found.extend(clippy_lints_in(group));
        }
        offset += line.len();
    }
    found
}

/// What one crate's sources say about the six lints.
#[derive(Default, Clone)]
struct CrateFacts {
    /// Crate root (repo-relative path) -> the lints its `#![deny(...)]` names.
    roots: BTreeMap<String, BTreeSet<String>>,
    /// Any file under `src/` -> the lints an `allow`/`expect`/`warn` attribute
    /// in it takes back.
    silenced: BTreeMap<String, BTreeSet<String>>,
}

/// Every `.rs` file under `dir`, sorted, so the walk is reproducible.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files
}

/// The crate roots of `crate_dir`: the lib, the bin beside it, and anything
/// under `src/bin/`. A root added later is picked up here without an edit, so
/// a new binary cannot enter the tree exempt.
fn crate_roots(crate_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for name in ["lib.rs", "main.rs"] {
        let path = crate_dir.join("src").join(name);
        if path.is_file() {
            roots.push(path);
        }
    }
    let bin_dir = crate_dir.join("src").join("bin");
    if bin_dir.is_dir() {
        roots.extend(rust_files(&bin_dir));
    }
    roots
}

/// Read the whole tree into the shape [`audit`] judges.
fn read_tree(root: &Path) -> BTreeMap<String, CrateFacts> {
    let crates_dir = root.join("crates");
    let entries = fs::read_dir(&crates_dir)
        .unwrap_or_else(|err| panic!("{} is not readable: {err}", crates_dir.display()));
    let mut tree: BTreeMap<String, CrateFacts> = BTreeMap::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.join("Cargo.toml").is_file() {
            continue;
        }
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| panic!("{} has no directory name", dir.display()));
        if OUT_OF_ENGINE.contains(&name.as_str()) {
            continue;
        }
        let relative = |path: &Path| {
            path.strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string()
                .replace('\\', "/")
        };
        let mut facts = CrateFacts::default();
        for path in crate_roots(&dir) {
            let text =
                fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            facts
                .roots
                .insert(relative(&path), lints_named_by(&text, &DENY_HEADS));
        }
        for path in rust_files(&dir.join("src")) {
            let text =
                fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            let silenced = lints_named_by(&text, &SILENCER_HEADS);
            if !silenced.is_empty() {
                facts.silenced.insert(relative(&path), silenced);
            }
        }
        tree.insert(name, facts);
    }
    tree
}

// ---------------------------------------------------------------------------
// The gate, as a pure function
// ---------------------------------------------------------------------------

/// `Ok(())` when the tree matches the table; otherwise the operator-facing
/// report, one fault per line.
fn audit(measured: &[(&str, [u32; 6])], tree: &BTreeMap<String, CrateFacts>) -> Result<(), String> {
    let mut faults: Vec<String> = Vec::new();
    let table: BTreeMap<&str, [u32; 6]> =
        measured.iter().map(|(name, row)| (*name, *row)).collect();

    for name in tree.keys() {
        if !table.contains_key(name.as_str()) {
            faults.push(format!(
                "{name} is a workspace crate with no row in MEASURED — measure it with the \
                 command in this file's header and add its row; a crate born after the \
                 measurement is not exempt from it"
            ));
        }
    }

    for (name, counts) in &table {
        let Some(facts) = tree.get(*name) else {
            faults.push(format!(
                "MEASURED has a row for {name}, which is not a crate in this workspace — \
                 delete the row"
            ));
            continue;
        };
        if facts.roots.is_empty() {
            faults.push(format!(
                "{name} has no crate root under src/ to carry its denies"
            ));
            continue;
        }
        for (index, lint) in LINTS.iter().enumerate() {
            let count = counts[index];
            for (path, denied) in &facts.roots {
                match (count, denied.contains(*lint)) {
                    (0, false) => faults.push(format!(
                        "{path} does not deny {lint}, which MEASURED prices at 0 for {name} — \
                         restore `#![deny({lint})]`, or, if the crate really did take a new \
                         site, raise the count in the same change and say why"
                    )),
                    (n, true) if n != 0 => faults.push(format!(
                        "{path} denies {lint} while MEASURED still prices it at {n} for {name} \
                         — a lint cannot be enabled without the measurement that says it costs \
                         nothing; re-measure and set the count to 0"
                    )),
                    _ => {}
                }
            }
            if count != 0 {
                continue;
            }
            for (path, silenced) in &facts.silenced {
                if silenced.contains(*lint) {
                    faults.push(format!(
                        "{path} takes {lint} back with an allow/expect/warn attribute, and \
                         {name} denies it at its root — the deny is the whole gate, so a \
                         per-site exemption is a hole in it"
                    ));
                }
            }
        }
    }

    if faults.is_empty() {
        return Ok(());
    }
    faults.sort();
    Err(format!(
        "the panic-shaped restriction lints are enabled per crate at the crate root, from the \
         measurement in crates/mush-contract-tests/tests/restriction_lints.rs:\n  {}",
        faults.join("\n  ")
    ))
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

/// The live gate: every crate root still denies what the table says it has
/// already paid for, and nothing under `src/` takes one of those denies back.
#[test]
fn every_crate_root_denies_the_lints_it_has_already_paid_for() {
    let tree = read_tree(&repo_root());
    if let Err(report) = audit(&MEASURED, &tree) {
        panic!("{report}");
    }
}

/// The census. `read_tree` walks the crates directory rather than a list, so a
/// new crate appears here on its own; this asserts the table grew with it, and
/// that the tree is the size the measurement covered.
#[test]
fn the_measurement_covers_every_crate_in_the_workspace() {
    let tree = read_tree(&repo_root());
    let measured: BTreeSet<&str> = MEASURED.iter().map(|(name, _)| *name).collect();
    let present: BTreeSet<&str> = tree.keys().map(String::as_str).collect();
    assert_eq!(
        measured, present,
        "MEASURED and crates/ have drifted apart; every workspace crate is measured, and only \
         workspace crates are"
    );
    assert_eq!(
        MEASURED.len(),
        43,
        "the measurement in this file's header covered 43 crates; S1 added mush-atlas and \
         mush-content, both born at zero on every lint; 16b cut R-b deleted ruleset-wod and \
         ruleset-shadowrun; the crates in OUT_OF_ENGINE are not measured"
    );
}

/// The enables are not decoration: at least one lint is denied, and the roster
/// is the one the measurement produced. A change that quietly emptied every
/// `#![deny]` would still satisfy a table of all-non-zero counts.
#[test]
fn the_enabled_set_is_the_one_the_measurement_produced() {
    let mut tally = [0usize; LINTS.len()];
    for (_, counts) in MEASURED {
        for (slot, count) in tally.iter_mut().zip(counts) {
            if count == 0 {
                *slot += 1;
            }
        }
    }
    let enabled: BTreeMap<&str, usize> = LINTS.into_iter().zip(tally).collect();
    assert_eq!(
        enabled,
        BTreeMap::from([
            // S1: mush-atlas and mush-content arrive at zero on every lint, so each
            // count is the measurement's plus two; WS01-S10B denies `expect_used` in
            // mush-session too, so that one is plus three.
            // 16b cut R-b: ruleset-wod (unwrap, panic_in_result_fn) and ruleset-shadowrun
            // (unwrap, panic, panic_in_result_fn) left with their denials.
            ("clippy::unwrap_used", 40),
            // 16b cut R-a: tacsim now carries the composition root's three expects, so
            // it no longer denies `expect_used`; nine crates do.
            ("clippy::expect_used", 9),
            ("clippy::indexing_slicing", 16),
            ("clippy::panic", 38),
            ("clippy::unreachable", 34),
            ("clippy::panic_in_result_fn", 41),
        ]),
        "the per-lint crate counts moved; re-measure before changing them"
    );
}

// ---------------------------------------------------------------------------
// The fixtures — what the gate refuses
// ---------------------------------------------------------------------------

/// One crate's facts from a literal roster, for the fixtures below.
fn facts(root: &str, denied: &[&str], silenced: &[(&str, &str)]) -> CrateFacts {
    CrateFacts {
        roots: BTreeMap::from([(
            root.to_string(),
            denied.iter().map(|lint| (*lint).to_string()).collect(),
        )]),
        silenced: silenced
            .iter()
            .map(|(path, lint)| ((*path).to_string(), BTreeSet::from([(*lint).to_string()])))
            .collect(),
    }
}

const CLEAN: [(&str, [u32; 6]); 1] = [("mush-widget", [0, 5, 0, 0, 0, 0])];

/// The happy path the fixtures below are perturbations of.
#[test]
fn a_root_that_denies_exactly_what_it_owes_passes() {
    let tree = BTreeMap::from([(
        "mush-widget".to_string(),
        facts(
            "crates/mush-widget/src/lib.rs",
            &[
                "clippy::unwrap_used",
                "clippy::indexing_slicing",
                "clippy::panic",
                "clippy::unreachable",
                "clippy::panic_in_result_fn",
            ],
            &[],
        ),
    )]);
    audit(&CLEAN, &tree).expect("a root that denies its five zeroes is in order");
}

/// The regression this file exists for: a deny quietly leaves a crate root and
/// the crate's floor is gone with it.
#[test]
fn a_root_that_drops_a_deny_is_refused() {
    let tree = BTreeMap::from([(
        "mush-widget".to_string(),
        facts(
            "crates/mush-widget/src/lib.rs",
            &[
                "clippy::unwrap_used",
                "clippy::indexing_slicing",
                "clippy::panic",
                "clippy::unreachable",
            ],
            &[],
        ),
    )]);
    let report = audit(&CLEAN, &tree).expect_err("a dropped deny is a hole in the gate");
    assert!(
        report.contains("does not deny clippy::panic_in_result_fn"),
        "{report}"
    );
}

/// The bypass: the deny stays at the root, and an `#[allow]` beside the new
/// `unwrap()` makes it inert. The gate reads the whole of `src/`, not just the
/// root, for exactly this.
#[test]
fn an_allow_that_takes_an_enabled_lint_back_is_refused() {
    let tree = BTreeMap::from([(
        "mush-widget".to_string(),
        facts(
            "crates/mush-widget/src/lib.rs",
            &[
                "clippy::unwrap_used",
                "clippy::indexing_slicing",
                "clippy::panic",
                "clippy::unreachable",
                "clippy::panic_in_result_fn",
            ],
            &[("crates/mush-widget/src/inner.rs", "clippy::unwrap_used")],
        ),
    )]);
    let report = audit(&CLEAN, &tree).expect_err("a per-site allow is a hole in the deny");
    assert!(
        report.contains("takes clippy::unwrap_used back"),
        "{report}"
    );
    // The lint the crate never enabled is nobody's business: `expect_used` is
    // priced at 5 here, so an allow of it is ordinary code.
    let allowed = BTreeMap::from([(
        "mush-widget".to_string(),
        facts(
            "crates/mush-widget/src/lib.rs",
            &[
                "clippy::unwrap_used",
                "clippy::indexing_slicing",
                "clippy::panic",
                "clippy::unreachable",
                "clippy::panic_in_result_fn",
            ],
            &[("crates/mush-widget/src/inner.rs", "clippy::expect_used")],
        ),
    )]);
    audit(&CLEAN, &allowed).expect("an allow of a lint this crate never enabled is not a bypass");
}

/// The other direction: a deny that the record does not back. Enabling a lint
/// is a measurement, and the number is how the next reader knows it was made.
#[test]
fn a_deny_the_measurement_does_not_back_is_refused() {
    let tree = BTreeMap::from([(
        "mush-widget".to_string(),
        facts(
            "crates/mush-widget/src/lib.rs",
            &[
                "clippy::unwrap_used",
                "clippy::expect_used",
                "clippy::indexing_slicing",
                "clippy::panic",
                "clippy::unreachable",
                "clippy::panic_in_result_fn",
            ],
            &[],
        ),
    )]);
    let report = audit(&CLEAN, &tree).expect_err("an unmeasured deny is refused");
    assert!(
        report.contains("denies clippy::expect_used while MEASURED still prices it at 5"),
        "{report}"
    );
}

/// A crate that enters the tree after the measurement is not exempt from it,
/// and a row for a crate that has left is not left behind.
#[test]
fn a_crate_the_measurement_never_saw_is_refused() {
    let tree = BTreeMap::from([
        (
            "mush-widget".to_string(),
            facts(
                "crates/mush-widget/src/lib.rs",
                &[
                    "clippy::unwrap_used",
                    "clippy::indexing_slicing",
                    "clippy::panic",
                    "clippy::unreachable",
                    "clippy::panic_in_result_fn",
                ],
                &[],
            ),
        ),
        (
            "mush-newcomer".to_string(),
            facts("crates/mush-newcomer/src/lib.rs", &[], &[]),
        ),
    ]);
    let report = audit(&CLEAN, &tree).expect_err("an unmeasured crate is refused");
    assert!(
        report.contains("mush-newcomer is a workspace crate with no row in MEASURED"),
        "{report}"
    );

    let gone = BTreeMap::new();
    let report = audit(&CLEAN, &gone).expect_err("a row for a departed crate is refused");
    assert!(
        report.contains("which is not a crate in this workspace"),
        "{report}"
    );
}

/// A second root in the same crate — the binary beside the lib — carries the
/// same denies. `mush-gateway`, `mush-server` and `mushd` each have one, and a
/// deny on the lib says nothing about the bin's own crate.
#[test]
fn a_second_root_in_the_same_crate_is_judged_too() {
    let mut widget = facts(
        "crates/mush-widget/src/lib.rs",
        &[
            "clippy::unwrap_used",
            "clippy::indexing_slicing",
            "clippy::panic",
            "clippy::unreachable",
            "clippy::panic_in_result_fn",
        ],
        &[],
    );
    widget.roots.insert(
        "crates/mush-widget/src/main.rs".to_string(),
        BTreeSet::from(["clippy::unwrap_used".to_string()]),
    );
    let tree = BTreeMap::from([("mush-widget".to_string(), widget)]);
    let report = audit(&CLEAN, &tree).expect_err("the bin root is a crate of its own");
    assert!(
        report.contains("crates/mush-widget/src/main.rs does not deny clippy::panic"),
        "{report}"
    );
}

// ---------------------------------------------------------------------------
// The reader
// ---------------------------------------------------------------------------

/// The attribute reader is the whole basis of the gate, so it is proven
/// against the shapes the tree actually holds: a multi-line deny, a one-line
/// one, an attribute quoted inside a doc comment, and — the one that would
/// wreck a naive scan — the `.expect("...")` method call, which is a paren
/// group after the word `expect` on nearly every page of this repo.
#[test]
fn the_attribute_reader_sees_attributes_and_not_prose() {
    let source = "\
//! A doc comment that quotes `#![deny(clippy::indexing_slicing)]` in prose.
//!
#![deny(
    clippy::unwrap_used,
    clippy::panic,
)]
#![deny(clippy::unreachable)]

fn body() -> usize {
    let value: Option<usize> = None;
    #[allow(clippy::expect_used)]
    let got = value.expect(\"clippy::panic_in_result_fn is not named by a string\");
    got
}
";
    assert_eq!(
        lints_named_by(source, &DENY_HEADS),
        BTreeSet::from([
            "clippy::unwrap_used".to_string(),
            "clippy::panic".to_string(),
            "clippy::unreachable".to_string(),
        ]),
        "the doc comment's attribute is prose, and both deny spellings are attributes"
    );
    assert_eq!(
        lints_named_by(source, &SILENCER_HEADS),
        BTreeSet::from(["clippy::expect_used".to_string()]),
        "an `.expect(\"...\")` call is not an attribute, whatever its argument says"
    );
}

/// An unterminated attribute must not swallow the rest of the file — nor panic
/// the gate that reads it.
#[test]
fn an_unclosed_attribute_names_nothing() {
    assert!(lints_named_by("#![deny(clippy::panic\n", &DENY_HEADS).is_empty());
    assert!(lints_named_by("#![deny]\n", &DENY_HEADS).is_empty());
}
