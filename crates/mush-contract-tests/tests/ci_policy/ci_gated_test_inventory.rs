//! **The gated-proof inventory** — a workspace policy gate (08j S4).
//!
//! A test that returns early when the `REDIS_URL` variable is unset, or that
//! carries `#[ignore]`, does not fail when nobody runs it: it *passes*,
//! everywhere, as a silent skip. K8.6 moved Redis out of the cross-platform
//! `test` job and named six proofs in `redis-faults`, which left nine
//! live-backend proofs owned by no job at all and reporting green on every
//! push.
//!
//! This test makes the invariant self-enforcing. It reads the source tree and
//! `.github/workflows/ci.yml` as data and asserts that **every backend-gated
//! test is reachable from a job that supplies the backend**:
//!
//! - a test is **backend-gated** when its own body reads the `REDIS_URL`
//!   environment variable, carries `#[ignore]`, or reads an opt-in environment
//!   flag (`RUN_*` / `*_RUN`) and returns early. Paid-provider opt-ins must
//!   have an explicit exemption below; a named job alone does not enable them.
//! - it is **covered** when `ci.yml` names the function verbatim, or (for an
//!   integration target) names that target as `-p <pkg> --test <target>`,
//!   the target's name ending there (`--test minds_rhai` is not `--test
//!   minds`) and with no test-name filter after it (`invokes_target`). A
//!   filtered step (`--test twin some_test -- --exact`, with or without
//!   `--ignored`) runs only the test it names, and covers that one by its
//!   name; it proves nothing about the other gated tests of the same file.
//!   That holds for a `REDIS_URL`-reading test as much as for an `#[ignore]`d
//!   one: the workspace sweep runs the former, but only its skip or in-memory
//!   branch — the branch that needs the backend exists only under a job that
//!   sets `REDIS_URL`, and a filtered step there runs only the test it names.
//!   (`clean_shutdown_drains_and_snapshots` in `mushd/tests/durable_process/durability_e2e.rs`
//!   was the case: `redis-faults` invoked its target only with its kill -9
//!   twin's name as the filter, so its Redis branch ran in no job at all.)
//!   A unit test under `src/` must be named verbatim: `-p <pkg> --lib` with a
//!   filter proves nothing about a *different* gated test in the same crate.
//!   A `cargo nextest run` line reads the same way (test-strategy cut T.1):
//!   its filterset (`-E 'test(=name)'`, `--filterset`) is a filter, and its
//!   own value-taking options (`--build-jobs 2`, `--run-ignored only`,
//!   `-P ci`) are not.
//! - since test-strategy cut T.3 a crate's integration tests may be modules
//!   of a **group binary** (`tests/<group>.rs` declaring `tests/<group>/<m>.rs`,
//!   its tests named `<m>::<fn>`): such a test is covered when a line names
//!   `-p <pkg> --test <group>` and runs the whole binary, or the whole module
//!   (`<m>::`, cargo's substring filter; nextest's `-E 'test(/^<m>::/)'`), or
//!   the test itself (`<m>::<fn>` with `--exact`; `-E 'test(=<m>::<fn>)'`)
//!   (`invokes_module`).
//!
//! Tests whose names merely end in `_gated` in the English sense
//! (`chown_is_power_gated`, `harm_is_staff_gated`) are not backend-gated: they
//! run, and fail, under the ordinary `cargo test --workspace` job. The suffix
//! is not the contract — skippability is.
//!
//! Since doc 43 cut TS.2 naming is not enough: every gated test also has a
//! row in `test-tiers.toml` giving its tier (`pr`, `nightly`, `weekly`,
//! `dispatch`, `by-hand`) and configuration, and the last tests of this file
//! hold each row to the workflows — where a comment is not a run, and an
//! ignored test runs only under `--ignored` / `--run-ignored`.
//!
//! Macro-generated tests (`mush-store`'s `redis_gated!` suite) are invisible to
//! a source scan by construction; their target is named by `redis-faults`
//! directly and the second test below pins the census so the scanner cannot
//! quietly stop finding anything.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use mush_contract_tests::scan;

/// The exact source form of a live-backend gate, built at runtime so that this
/// file does not match its own needle.
fn redis_gate_needle() -> String {
    format!("env::var({:?})", "REDIS_URL")
}

/// One discovered test function.
#[derive(Debug)]
struct DiscoveredTest {
    name: String,
    /// Path relative to `crates/`, e.g. `mushd/tests/durable_process/persistence.rs`.
    relative: String,
    ignored: bool,
    /// The test's own body reads the live-backend variable.
    reads_live_backend: bool,
    /// Explicit opt-in flags in an early-returning test body.
    opt_in_flags: BTreeSet<String>,
    /// Every variable an early-returning test body reads by literal name: the
    /// candidates for an opt-in `test-tiers.toml` declares (doc 43 TS.18).
    early_return_reads: BTreeSet<String>,
}

impl DiscoveredTest {
    fn backend_gated(&self) -> bool {
        self.ignored || self.reads_live_backend || !self.opt_in_flags.is_empty()
    }

    /// `Some((package, target, module))`: `crates/<pkg>/tests/<target>.rs` is
    /// its own cargo target (`module` is `None`), and since test-strategy cut
    /// T.3 `crates/<pkg>/tests/<target>/<m>.rs` (or deeper) is a module of the
    /// group binary `tests/<target>.rs` when that root exists, its tests named
    /// `<m>::<fn>` (`<m>::<child>::<fn>` a level down). Anything else under
    /// `tests/` (`support/`, `common/mod.rs`) is no target of its own: its
    /// tests are covered by name only.
    fn integration_target(&self, root: &Path) -> Option<(&str, &str, Option<String>)> {
        let (package, rest) = self.relative.split_once('/')?;
        let inside = rest.strip_prefix("tests/")?.strip_suffix(".rs")?;
        let Some((target, below)) = inside.split_once('/') else {
            return Some((package, inside, None));
        };
        let group_root = root
            .join("crates")
            .join(package)
            .join("tests")
            .join(format!("{target}.rs"));
        if !group_root.is_file() {
            return None;
        }
        let module = below
            .strip_suffix("/mod")
            .unwrap_or(below)
            .replace('/', "::");
        Some((package, target, Some(module)))
    }
}

/// Opt-in spelling is part of this source gate's contract. Knobs such as a
/// duration or report path do not make a test optional.
fn opt_in_flags(body: &str) -> BTreeSet<String> {
    early_return_reads(body)
        .into_iter()
        .filter(|name| name.starts_with("RUN_") || name.ends_with("_RUN"))
        .collect()
}

/// The variables an early-returning body reads by literal name.
fn early_return_reads(body: &str) -> BTreeSet<String> {
    if !body
        .lines()
        .any(|line| line.trim_start().starts_with("return"))
    {
        return BTreeSet::new();
    }
    let needle = format!("env::var({}", '"');
    body.split(&needle)
        .skip(1)
        .filter_map(|tail| tail.split_once('"').map(|(name, _)| name))
        .map(str::to_owned)
        .collect()
}

/// Paid network calls require the operator's explicit opt-in. The cost report
/// also runs under its exact CI filter with `TACSIM_LLM_COST_MOCK=1`; that
/// exercises the report, not the live-provider exemption.
const PAID_OPT_INS: &[(&str, &str, &str, &str)] = &[
    (
        "tacsim/tests/harness.rs",
        "tacsim_harness_llm_cost_run",
        "TACSIM_LLM_COST_RUN",
        "Owner-funded cost measurement; CI uses the mock provider.",
    ),
    (
        "mushd/tests/world_journeys/npc_smoke.rs",
        "live_npc_dialogue_openrouter_smoke",
        "RUN_LLM_SMOKE",
        "Manual live-provider smoke; needs an operator-funded API key.",
    ),
    (
        "mushd/tests/world_journeys/worldgen_smoke.rs",
        "live_worldgen_openrouter_smoke",
        "RUN_LLM_SMOKE",
        "Manual live-provider smoke; needs an operator-funded API key.",
    ),
];

/// The identifier of a function-defining line, if it is one.
fn function_name(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("pub async fn ")
        .or_else(|| line.strip_prefix("pub fn "))
        .or_else(|| line.strip_prefix("async fn "))
        .or_else(|| line.strip_prefix("fn "))?;
    let (name, _) = rest.split_once('(')?;
    let name = name.trim();
    // `$name` inside a `macro_rules!` body is not a test the scanner can see.
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(name)
}

/// The leading whitespace of a line, which rustfmt makes a reliable block key.
fn indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Scan one file for `#[test]` / `#[tokio::test]` functions and their bodies.
///
/// Deliberately a line scanner, not a parser: the tree is rustfmt-clean, so a
/// function ends at the first line whose content is `}` at the function's own
/// indentation.
fn scan_file(path: &Path, relative: &str, out: &mut Vec<DiscoveredTest>) {
    let Ok(source) = fs::read_to_string(path) else {
        return;
    };
    let needle = redis_gate_needle();
    let lines: Vec<&str> = source.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if !(trimmed.starts_with("#[test]") || trimmed.starts_with("#[tokio::test")) {
            index += 1;
            continue;
        }
        let mut cursor = index + 1;
        let mut ignored = false;
        while cursor < lines.len() {
            let attribute = lines[cursor].trim();
            if attribute.starts_with("#[ignore") {
                ignored = true;
            } else if !(attribute.starts_with("#[")
                || attribute.starts_with("//")
                || attribute.is_empty())
            {
                break;
            }
            cursor += 1;
        }
        let Some(name) = lines
            .get(cursor)
            .and_then(|line| function_name(line.trim()))
        else {
            index = cursor + 1;
            continue;
        };
        let opening = indent(lines[cursor]).to_string();
        let mut end = cursor;
        let mut body = String::new();
        while end < lines.len() {
            body.push_str(lines[end]);
            body.push('\n');
            if end > cursor && lines[end].trim() == "}" && indent(lines[end]) == opening {
                break;
            }
            end += 1;
        }
        out.push(DiscoveredTest {
            name: name.to_string(),
            relative: relative.to_string(),
            ignored,
            reads_live_backend: body.contains(&needle),
            opt_in_flags: opt_in_flags(&body),
            early_return_reads: early_return_reads(&body),
        });
        index = end + 1;
    }
}

fn discovered_tests(root: &Path) -> Vec<DiscoveredTest> {
    let crates_dir = root.join("crates");
    let files = scan::files(&crates_dir, &["rs"], &["target"]);
    let mut tests = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(&crates_dir)
            .expect("scanned under crates/")
            .to_string_lossy()
            .replace('\\', "/");
        scan_file(file, &relative, &mut tests);
    }
    tests
}

/// Cargo's and nextest's own options (before `--`) that take a value as the
/// next word, so the value is not read as a test-name filter. `-j` is cargo's
/// build jobs and nextest's test threads: a value either way.
const VALUE_OPTIONS: [&str; 22] = [
    "-p",
    "--package",
    "--test",
    "--features",
    "-F",
    "--target",
    "--profile",
    "-j",
    "--jobs",
    "--bin",
    "--example",
    "--bench",
    "--manifest-path",
    "--color",
    // nextest's.
    "-P",
    "--build-jobs",
    "--test-threads",
    "--run-ignored",
    "--partition",
    "--cargo-profile",
    "--config-file",
    "--tool-config-file",
];

/// nextest's filterset options: the expression they take is a test filter,
/// however it is spelled (`-E 'test(=name)'`, `-E'test(name)'`,
/// `--filterset=...`).
const FILTERSET_OPTIONS: [&str; 2] = ["-E", "--filterset"];

/// **Whether `workflow` invokes the integration target** `<target>` of
/// `<package>` (17b cut M2.7): some line holds `-p <package> --test
/// <target>` with the target's name ending there (whitespace or the line's
/// end — so `--test minds_rhai` is not `--test minds`); and, when `whole`,
/// with no test-name filter among cargo's own arguments after it. A filtered
/// invocation (`--test twin some_test -- --ignored --exact`, or a bare
/// substring filter) runs only the tests it names, which are covered by
/// their verbatim names: a named single-test step must not cover every
/// other gated test of its file. The policy test asks for `whole` for every
/// backend-gated test, the `REDIS_URL`-reading ones included: the workspace
/// sweep runs those, but only their skip or in-memory branch, and the branch
/// that needs the backend runs only under a job that sets `REDIS_URL` — where
/// a filtered step is as blind to the file's other tests as it is for an
/// `#[ignore]`d one. (`whole = false` answers only whether the target is
/// named at all; the unit test below pins the difference.) A `cargo nextest
/// run` line is read the same way: a filterset is a filter, and nextest's
/// value-taking options are skipped with their values.
fn invokes_target(workflow: &str, package: &str, target: &str, whole: bool) -> bool {
    let needle = format!("-p {package} --test {target}");
    workflow.lines().any(|line| {
        line.match_indices(&needle).any(|(at, _)| {
            let rest = line.get(at + needle.len()..).unwrap_or_default();
            if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
                return false;
            }
            if !whole {
                return true;
            }
            let mut words = rest.split_whitespace();
            while let Some(word) = words.next() {
                if word == "--" {
                    break;
                }
                if word.starts_with('-') {
                    if FILTERSET_OPTIONS
                        .iter()
                        .any(|option| word.starts_with(option))
                    {
                        return false;
                    }
                    if VALUE_OPTIONS.contains(&word) {
                        let _ = words.next();
                    }
                    continue;
                }
                // A positional word before `--` is a test-name filter.
                return false;
            }
            true
        })
    })
}

#[test]
fn a_gated_test_is_covered_only_by_an_unfiltered_invocation_of_its_target() {
    let whole = "run: cargo test -p mushd --test persistence -- --test-threads=1";
    assert!(invokes_target(whole, "mushd", "persistence", true));
    assert!(invokes_target(
        "cargo test -p mush-proto --test ts_export --features ts",
        "mush-proto",
        "ts_export",
        true
    ));
    assert!(invokes_target(
        "cargo test -p tacsim --test harness",
        "tacsim",
        "harness",
        true
    ));
    // A longer target's name is not this one's, filtered or not.
    let named = "run: cargo test -p tacsim --test minds_rhai m2_one -- --ignored --exact";
    for whole in [true, false] {
        assert!(!invokes_target(named, "tacsim", "minds", whole));
    }
    // A filtered invocation covers only the test it names (by its name) when
    // the whole target is asked for, and names the target otherwise.
    assert!(!invokes_target(named, "tacsim", "minds_rhai", true));
    assert!(invokes_target(named, "tacsim", "minds_rhai", false));
    assert!(!invokes_target(
        "cargo test -p mush-bot --test arena_win_rate some_test -- --ignored",
        "mush-bot",
        "arena_win_rate",
        true
    ));
    // nextest's spelling: its value-taking options are not filters, and a
    // filterset is one, with or without a space before the expression.
    assert!(invokes_target(
        "cargo nextest run -P ci -p mushd --test persistence --build-jobs 2 --run-ignored only",
        "mushd",
        "persistence",
        true
    ));
    for filtered in [
        "cargo nextest run -p tacsim --test twin -E 'test(=m2_one)' --run-ignored only",
        "cargo nextest run -p tacsim --test twin -E'test(m2_one)'",
        "cargo nextest run -p tacsim --test twin --filterset 'test(/m2_/)'",
        "cargo nextest run -p tacsim --test twin m2_one --build-jobs 2 -- --ignored --exact",
    ] {
        assert!(
            !invokes_target(filtered, "tacsim", "twin", true),
            "{filtered}"
        );
        assert!(
            invokes_target(filtered, "tacsim", "twin", false),
            "{filtered}"
        );
    }
}

/// **Whether `workflow` runs the test `test_id` (`<module>::<fn>`) of the
/// group binary `<target>` of `<package>`** (test-strategy cut T.3): some
/// line holds `-p <package> --test <target>`, the target's name ending
/// there, and the test is within what that line runs — the whole binary (no
/// filter at all), or a filter it satisfies. A positional word before `--`
/// is cargo's substring filter on the full test name (`<module>::` is the
/// whole-module spelling; after `-- --exact` it must equal the id); a
/// nextest filterset covers it as `test(/^<module>::/)` or `test(=<id>)`,
/// the two spellings the tree uses (an expression with spaces inside its
/// quotes is read as its first word, and covers nothing).
fn invokes_module(workflow: &str, package: &str, target: &str, test_id: &str) -> bool {
    let needle = format!("-p {package} --test {target}");
    let module = test_id.split("::").next().unwrap_or(test_id);
    let module_filterset = format!("test(/^{module}::/)");
    let exact_filterset = format!("test(={test_id})");
    workflow.lines().any(|line| {
        line.match_indices(&needle).any(|(at, _)| {
            let rest = line.get(at + needle.len()..).unwrap_or_default();
            if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
                return false;
            }
            let exact = rest
                .split_whitespace()
                .skip_while(|word| *word != "--")
                .any(|word| word == "--exact");
            let mut filters: Vec<&str> = Vec::new();
            let mut filtersets: Vec<&str> = Vec::new();
            let mut words = rest.split_whitespace();
            while let Some(word) = words.next() {
                if word == "--" {
                    break;
                }
                if word.starts_with('-') {
                    if let Some(option) = FILTERSET_OPTIONS
                        .iter()
                        .find(|option| word.starts_with(*option))
                    {
                        let attached = word[option.len()..].trim_start_matches('=');
                        let expression = if attached.is_empty() {
                            words.next().unwrap_or_default()
                        } else {
                            attached
                        };
                        filtersets.push(expression.trim_matches(|c| c == '\'' || c == '"'));
                        continue;
                    }
                    if VALUE_OPTIONS.contains(&word) {
                        let _ = words.next();
                    }
                    continue;
                }
                filters.push(word);
            }
            let by_filter = if filters.is_empty() {
                true
            } else if exact {
                filters.contains(&test_id)
            } else {
                filters.iter().any(|filter| test_id.contains(filter))
            };
            let by_filterset = filtersets.is_empty()
                || filtersets.iter().any(|expression| {
                    *expression == module_filterset || *expression == exact_filterset
                });
            by_filter && by_filterset
        })
    })
}

#[test]
fn a_module_of_a_group_binary_is_covered_by_the_binary_the_module_or_the_test() {
    let id = "persistence::kill9_mid_play_world_survives_redis_gated";
    let other = "m10c_e2e::m10c_persistence_twin_redis_gated";
    // The whole binary, and the whole module (cargo's substring filter).
    assert!(invokes_module(
        "run: cargo test -p mushd --test durable_process -- --test-threads=1",
        "mushd",
        "durable_process",
        id
    ));
    let module =
        "run: cargo test -p mushd --test durable_process persistence:: -- --test-threads=1";
    assert!(invokes_module(module, "mushd", "durable_process", id));
    assert!(!invokes_module(module, "mushd", "durable_process", other));
    // A substring is a substring: `lane::` selects `cost_lane::…` too, as
    // cargo would (which is why a group refuses such a pair of modules).
    assert!(invokes_module(
        "cargo test -p tacsim --test shard_4 lane::",
        "tacsim",
        "shard_4",
        "cost_lane::x"
    ));
    // `--exact` wants the full id; the module spelling alone then runs nothing.
    let exact = format!("run: cargo test -p mushd --test durable_process {id} -- --exact");
    assert!(invokes_module(&exact, "mushd", "durable_process", id));
    assert!(!invokes_module(
        &exact,
        "mushd",
        "durable_process",
        "persistence::clean_shutdown_drains_and_snapshots"
    ));
    assert!(!invokes_module(
        "run: cargo test -p mushd --test durable_process persistence:: -- --exact",
        "mushd",
        "durable_process",
        id
    ));
    // nextest's spellings, and a longer target's name is not this one's.
    assert!(invokes_module(
        "cargo nextest run -P ci -p mushd --test durable_process -E 'test(/^persistence::/)' --build-jobs 2",
        "mushd",
        "durable_process",
        id
    ));
    assert!(invokes_module(
        &format!("cargo nextest run -p mushd --test durable_process -E'test(={id})'"),
        "mushd",
        "durable_process",
        id
    ));
    assert!(!invokes_module(
        "cargo nextest run -p mushd --test durable_process -E 'test(/^m10c_e2e::/)'",
        "mushd",
        "durable_process",
        id
    ));
    assert!(!invokes_module(
        "cargo test -p mushd --test durable_process_x",
        "mushd",
        "durable_process",
        id
    ));
}

#[test]
fn every_backend_gated_proof_is_named_by_a_ci_job() {
    let root = scan::repo_root();
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("the CI workflow is part of the tree this policy governs");
    let tests = discovered_tests(&root);
    assert!(
        tests.len() > 500,
        "the scanner found only {} tests — it is broken, not the tree",
        tests.len()
    );

    let gated: Vec<&DiscoveredTest> = tests.iter().filter(|t| t.backend_gated()).collect();
    assert!(
        gated.len() >= 10,
        "expected at least the ten live-backend proofs, found {}",
        gated.len()
    );

    let mut unowned: BTreeSet<String> = BTreeSet::new();
    for test in gated {
        if !test.opt_in_flags.is_empty() {
            for flag in &test.opt_in_flags {
                if !PAID_OPT_INS
                    .iter()
                    .any(|(path, name, allowed_flag, reason)| {
                        *path == test.relative
                            && *name == test.name
                            && *allowed_flag == flag
                            && !reason.is_empty()
                    })
                {
                    unowned.insert(format!(
                        "{} ({}) requires {flag}: add explicit ownership/exemption",
                        test.name, test.relative
                    ));
                }
            }
            continue;
        }
        if workflow.contains(&test.name) {
            continue;
        }
        // The whole target, filter-free, for every gated test: a filtered
        // step covers only the test it names, by its name (checked above). A
        // module of a group binary: the whole binary, the whole module, or
        // the test itself (`invokes_module`).
        match test.integration_target(&root) {
            Some((package, target, None)) if invokes_target(&workflow, package, target, true) => {
                continue;
            }
            Some((package, target, Some(module)))
                if invokes_module(
                    &workflow,
                    package,
                    target,
                    &format!("{module}::{}", test.name),
                ) =>
            {
                continue;
            }
            _ => {}
        }
        unowned.insert(format!("{} ({})", test.name, test.relative));
    }

    assert!(
        unowned.is_empty(),
        "these proofs skip themselves without a live backend and no CI job runs them — \
         name each one (or its `-p <pkg> --test <target>`, with no test-name filter; for a \
         module of a group binary, `--test <group> <module>::`) in .github/workflows/ci.yml:\n  {}",
        unowned.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

#[test]
fn paid_opt_in_exemptions_are_live_and_the_cost_report_has_a_mock_ci_run() {
    let root = scan::repo_root();
    let tests = discovered_tests(&root);
    for (path, name, flag, reason) in PAID_OPT_INS {
        assert!(!reason.is_empty());
        assert!(
            tests.iter().any(|test| test.relative == *path
                && test.name == *name
                && test.opt_in_flags.contains(*flag)),
            "stale paid-provider exemption: {path}::{name} ({flag})"
        );
    }
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let mock_step = workflow
        .split("- name: The LLM cost report with a mock provider")
        .nth(1)
        .expect("the cost report has an explicit CI step")
        .split("- name:")
        .next()
        .unwrap();
    for needle in [
        "TACSIM_LLM_COST_RUN: \"1\"",
        "TACSIM_LLM_COST_MOCK: \"1\"",
        "cargo test -p tacsim --test harness tacsim_harness_llm_cost_run -- --exact",
    ] {
        assert!(
            mock_step.contains(needle),
            "mock cost step is missing {needle}"
        );
    }
    // A newly introduced opt-in cannot disappear merely because its test name
    // has no `_gated` suffix, and a duration knob is not an opt-in.
    assert_eq!(
        opt_in_flags(&format!(
            "if {} {{\n return;\n}}",
            "std::env::var(\"RUN_NEW_PROVIDER\").is_err()"
        )),
        BTreeSet::from(["RUN_NEW_PROVIDER".to_string()])
    );
    assert!(opt_in_flags("std::env::var(\"TACSIM_UNITS\");\nreturn;").is_empty());
}

/// The companion half: the `*_gated` suffix stays the name a live-backend proof
/// wears, and the census cannot quietly empty itself behind a rename.
#[test]
fn the_gated_suffix_census_is_stable_and_every_live_backend_proof_names_its_backend() {
    let root = scan::repo_root();
    let tests = discovered_tests(&root);
    let live: BTreeSet<&str> = tests
        .iter()
        .filter(|t| t.backend_gated() && t.name.ends_with("_gated"))
        .map(|t| t.name.as_str())
        .collect();
    // 10 at the 08j S4 fold; +1 for H1's `scene_archive_reads_cold_chunks_redis_gated`;
    // +2 for H3's `scale_world_boot_redis_gated` and `scale_authoritative_soak_redis_gated`;
    // +2 for H4's `economy_journal_round_trip_redis_gated` and
    // `inventory_pool_round_trip_redis_gated`; -2 for 16b cut R-b, whose deleted
    // character rulesets took their two sheet-reboot proofs with them; +1 for
    // R2's relay queue and frozen-leg process-death proof.
    assert_eq!(
        live.len(),
        14,
        "the live-backend `*_gated` census changed: {live:?}"
    );
    for name in &live {
        assert!(
            name.contains("redis"),
            "a live-backend proof must say which backend it needs: {name}"
        );
    }
}

// ---- E7 cut 11 independent verify fold ----
//
// **G5/G6/G15/F7: 10b §14 becomes a gate rather than prose.** §14's own rule is
// *"a migration commit cites the SYMBOLS below, never these numbers"* — and
// three of its four censused lanes cited a job that cannot see the crate that
// owns the lane, or a test cell that does not exist:
//
//   * **L4** cited `cargo test -p mush-contract-tests --test ci_policy schedule_source::`
//     for `industry.workorder`, and this package declares no `mush-industry`
//     dependency at all — its three toy-fixture tests name neither "production"
//     nor "workorder", so L4's go criterion was dischargeable GREEN while
//     proving nothing about the migrated family;
//   * **L3** cited the H3 scale harness, whose only tick-shaped cell measures a
//     synthetic `ReplayWrite` count from env vars and names no `mush-sim` or
//     `mush-tactics` symbol;
//   * **L5/L6** cited a `gate5_fault_matrix` "partial-materialization cell"
//     that does not exist — the file has one test, in-process, over an
//     `AuthoritativeFaultStore`, and asserts no such criterion.
//
// The two assertions below are what turn §14 into something a reviewer who was
// not here can execute: for every lane §14 names, the cited invocation's package
// must declare the crate that owns the lane, and every proof §14 names must
// exist by name in the file it is attributed to.

/// One lane of 10b §14.2, with the crate that owns it and the CI invocation
/// §14 cites as its lane proof.
struct GateLane {
    /// §14.2's label.
    lane: &'static str,
    /// The crate whose source the migration re-points.
    owner_crate: &'static str,
    /// The invocation §14.2 names, as `-p <pkg> --test <target>` (or `--lib`).
    invocation: &'static str,
    /// The package that invocation runs in.
    invoked_package: &'static str,
}

/// Every manifest line that declares a workspace dependency, for one package.
fn declares(root: &Path, package: &str, wanted: &str) -> bool {
    let manifest = fs::read_to_string(root.join("crates").join(package).join("Cargo.toml"))
        .unwrap_or_else(|_| panic!("crates/{package}/Cargo.toml"));
    manifest.lines().any(|line| {
        let code = line.trim_start();
        !code.starts_with('#')
            && (code.starts_with(wanted) || code.starts_with(&format!("{wanted} ")))
    })
}

#[test]
fn vf_g5_every_e7k_lane_job_can_see_the_crate_that_owns_the_lane() {
    let root = scan::repo_root();
    // 10b §14.2, as corrected by the independent verify fold.
    let lanes = [
        GateLane {
            lane: "L1+L2 sim.placement + sim.station_assignment",
            owner_crate: "mush-sim",
            invocation: "-p mush-sim --test e7c11",
            invoked_package: "mush-sim",
        },
        GateLane {
            lane: "L1+L2 (the cross-crate consumers)",
            owner_crate: "mush-sim",
            invocation: "-p mush-campaign --test e7c10",
            invoked_package: "mush-campaign",
        },
        GateLane {
            lane: "L1+L2 (the class change)",
            owner_crate: "mush-sim",
            invocation: "-p mush-server --test world_task production_action_migrations::",
            invoked_package: "mush-server",
        },
        GateLane {
            lane: "L3 tactics.unit",
            owner_crate: "mush-tactics",
            invocation: "-p mush-tactics --test e7c11",
            invoked_package: "mush-tactics",
        },
        GateLane {
            lane: "L4 industry.workorder",
            owner_crate: "mush-industry",
            invocation: "-p mush-industry --test e7c4",
            invoked_package: "mush-industry",
        },
        GateLane {
            lane: "L5 the ledger append",
            owner_crate: "mush-economy",
            invocation: "-p mush-contract-tests --test ci_policy gate5_fault_matrix::",
            invoked_package: "mush-contract-tests",
        },
        GateLane {
            lane: "L6 cross-owner materialization",
            owner_crate: "mush-industry",
            invocation: "-p mush-industry --test e7c3",
            invoked_package: "mush-industry",
        },
    ];

    let mut blind: Vec<String> = Vec::new();
    for lane in &lanes {
        let sees = lane.invoked_package == lane.owner_crate
            || declares(&root, lane.invoked_package, lane.owner_crate);
        if !sees {
            blind.push(format!(
                "{}: `cargo test {}` runs in `{}`, which declares no dependency on \
                 `{}` and so cannot link the crate that owns the lane",
                lane.lane, lane.invocation, lane.invoked_package, lane.owner_crate
            ));
        }
    }
    assert!(
        blind.is_empty(),
        "10b §14.2 names a job that cannot see its lane:\n  {}",
        blind.join("\n  ")
    );

    // The citation the fold REMOVED, pinned as removed: `schedule_source` runs
    // in this package, which declares no `mush-industry` at all. If a future
    // edit re-adds the dependency the citation could come back — and that is a
    // decision, not an accident.
    assert!(
        !declares(&root, "mush-contract-tests", "mush-industry"),
        "`mush-contract-tests` gained a `mush-industry` dependency: 10b §14.2's \
         L4 citation may be revisited, deliberately"
    );

    // §14.3 criterion 4: the lane proofs run on every CI run, so a reviewer
    // running CI runs them. They were named line by line in the *K8 kernel
    // contracts* job until doc 43 cut TS.1 deleted it as a re-run of the
    // `test` matrix; what is pinned now is that the matrix cells which run
    // them still do: rest-libs' workspace sweep excludes neither
    // `mush-industry` (e7c4) nor `mush-contract-tests` (gate5_fault_matrix),
    // and rest-server's sweep names mush-server's `world_task` binary
    // (production_action_migrations::).
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("the CI workflow is part of the tree this policy governs");
    let sweep = |needle: &str| {
        workflow
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with(needle))
            .unwrap_or_else(|| {
                panic!("10b §14.3 criterion 4: ci.yml no longer runs `{needle} ...`")
            })
    };
    let rest_libs = sweep("cargo nextest run -P ci --workspace");
    for package in ["mush-industry", "mush-contract-tests"] {
        assert!(
            !rest_libs
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair == ["--exclude", package]),
            "10b §14.3 criterion 4: the rest-libs sweep excludes {package}, so its lane \
             proof runs in no CI job"
        );
    }
    let rest_server = sweep("cargo nextest run -P ci -p mush-server --test commerce_composition");
    assert!(
        rest_server
            .split_whitespace()
            .any(|word| word == "world_task"),
        "10b §14.3 criterion 4: the rest-server sweep no longer runs mush-server's \
         world_task binary (production_action_migrations::)"
    );
}

#[test]
fn vf_g6_every_proof_10b_section_14_names_exists_in_the_file_it_is_attributed_to() {
    let root = scan::repo_root();
    // (file relative to `crates/`, the test name §14 cites).
    let cited: [(&str, &str); 9] = [
        // L1+L2's go criterion.
        (
            "mush-campaign/tests/e7c10.rs",
            "e7c10_p1_no_tacsim_party_type_exists",
        ),
        (
            "mush-campaign/tests/e7c10.rs",
            "e7c10_p2_no_legality_check_reads_a_seat_grant",
        ),
        (
            "mush-sim/tests/e7c10.rs",
            "e7c10_p5_the_sim_export_surface_is_exactly_two_seam_functions",
        ),
        // L1+L2's writer censuses (the fold's).
        (
            "mush-sim/tests/e7c11.rs",
            "vf_g12_the_placement_writer_census_is_pinned",
        ),
        (
            "mush-sim/tests/e7c11.rs",
            "vf_g13_the_station_assignment_writer_census_is_pinned",
        ),
        // L3's writer census, and its capacity measurement.
        (
            "mush-tactics/tests/e7c11.rs",
            "vf_g4_the_unit_store_writer_census_is_pinned",
        ),
        // L4's lane proof: the `industry.workorder.completed` marker.
        (
            "mush-industry/tests/e7c4.rs",
            "e7c4_b7_the_completion_marker_round_trips_and_names_its_fact",
        ),
        (
            "mush-industry/tests/e7c11.rs",
            "vf_g13_l4_the_work_order_writer_census_is_pinned",
        ),
        // L5/L6's fault proof, named exactly as it exists.
        (
            "mush-contract-tests/tests/ci_policy/gate5_fault_matrix.rs",
            "gate1_tactical_and_delayed_lifecycle_bundles_survive_every_in_process_cut",
        ),
    ];
    let mut missing: Vec<String> = Vec::new();
    for (relative, name) in cited {
        let text = fs::read_to_string(root.join("crates").join(relative))
            .unwrap_or_else(|_| panic!("crates/{relative}"));
        if !text.contains(&format!("fn {name}(")) {
            missing.push(format!("{name} (attributed to crates/{relative})"));
        }
    }
    assert!(
        missing.is_empty(),
        "10b §14 cites a proof that does not exist where it says it does:\n  {}",
        missing.join("\n  ")
    );

    // **G6, the negative half.** §14.2's L6 cited a `gate5_fault_matrix`
    // "partial-materialization cell". There is none: the file's one test cuts
    // after every materialization op of an in-process `AuthoritativeFaultStore`
    // bundle and asserts no such criterion. §14 now names it as a cell the
    // MIGRATION commit must add, and this pins that it has not been added yet —
    // so the day it is, §14's wording moves with it.
    let matrix = fs::read_to_string(
        root.join("crates/mush-contract-tests/tests/ci_policy/gate5_fault_matrix.rs"),
    )
    .expect("gate5_fault_matrix.rs");
    assert!(
        !matrix.to_lowercase().contains("fn partial_materializ"),
        "a partial-materialization cell now exists: 10b §14.2's L5/L6 citation \
         must be changed from \"the migration commit must add\" to naming it"
    );
    assert_eq!(
        matrix.matches("#[tokio::test]").count() + matrix.matches("#[test]").count(),
        1,
        "gate5_fault_matrix carries exactly one test cell"
    );

    // **G15, the negative half.** §14.2's L3 cited "the H3 scale harness job".
    // Its one tick-shaped cell measures a SYNTHETIC bundle shape from env-var
    // counts and names no `mush-sim` or `mush-tactics` symbol, so it proves
    // generic bundle throughput and not the lane. §14 now cites
    // `-p mush-sim --test e7c8` for the measurement instead.
    let harness = fs::read_to_string(
        root.join("crates/mush-contract-tests/tests/ci_policy/scale_harness.rs"),
    )
    .expect("scale_harness.rs");
    assert!(
        harness.contains("fn scale_tick_shaped_bundle_measures_caps("),
        "the tick-shaped cell still exists, and still measures a synthetic shape"
    );
    assert!(
        !harness.contains("mush_sim::") && !harness.contains("mush_tactics::"),
        "the H3 harness names no symbol of the lane it was cited for"
    );
    let e7c8 = fs::read_to_string(root.join("crates/mush-sim/tests/e7c8.rs")).expect("e7c8.rs");
    assert!(
        e7c8.contains("measure_tick_bundle") && e7c8.contains("check_domain_capacity"),
        "…and the measurement §14's L3 go criterion needs really does run here"
    );
}

// ---- Doc 43 cut TS.2: every gated test declares its tier ----
//
// The census above asks only that ci.yml *names* a gated test somewhere. Two
// holes followed (doc 43 §5.9, §6 item 6): a comment counted as a name, so a
// test "run by hand" was green with no recorded run; and nothing said when a
// named job runs — on every pull request, nightly, or only when somebody
// presses the button. `test-tiers.toml` is the declaration: one row per gated
// test (or per file, `<key prefix>::*`) and configuration, with its tier, and
// these tests hold the rows to the workflows.

/// The manifest, beside this package's other registries.
const TIERS_MANIFEST: &str = "test-tiers.toml";

/// The tiers, most frequent first (doc 43 TS.2).
const TIERS: [&str; 5] = ["pr", "nightly", "weekly", "dispatch", "by-hand"];

/// The Sunday cron (D-43-11). A job whose `if:` admits this schedule is weekly.
const WEEKLY_CRON: &str = "17 4 * * 0";

#[derive(Debug, serde::Deserialize)]
struct TierManifest {
    #[serde(rename = "test")]
    rows: Vec<TierRow>,
    /// Doc 43 cut TS.18: the variables tacsim's and the bot's tests read that
    /// are not a configuration of their own (or, `opt-in`, make a test gated).
    #[serde(rename = "knob", default)]
    knobs: Vec<Knob>,
}

/// The kinds of [`Knob`]: why a variable a test reads needs no row of its own.
const KNOB_KINDS: [(&str, &str); 7] = [
    (
        "opt-in",
        "an early return unless set: the test is gated and has rows",
    ),
    (
        "size",
        "a sizing knob: the default is what the rows cover, another value a recorded measurement",
    ),
    (
        "measurement",
        "changes what is measured, never what is asserted",
    ),
    ("report", "where a report or preview is written"),
    ("bless", "rewrites a committed fixture or baseline"),
    (
        "secret",
        "a credential, or a name a test keeps unset on purpose",
    ),
    (
        "plumbing",
        "set by a parent process or script for its child",
    ),
];

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Knob {
    name: String,
    kind: String,
    note: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TierRow {
    /// `<pkg>/tests::<module or target>::<fn>` for an integration test (the
    /// group directory left out, so a reshard rewrites no row), `<path under
    /// crates/>::<fn>` otherwise; `<prefix>::*` for every gated test under it.
    id: String,
    tier: String,
    /// Space-separated `NAME` or `NAME=VALUE`: the environment this row's run
    /// sets. Empty is the default configuration.
    #[serde(default)]
    config: String,
    /// The ci.yml job that runs it, or `gate.yml:<kind>` for the VPS gate.
    #[serde(default)]
    job: String,
    /// A script the job runs that runs the test (the test is named in the
    /// script, and in the job at most in a comment).
    #[serde(default)]
    via: String,
    /// The `by-hand:` tag: the revision of the last recorded run, or `never`
    /// (with a `note` saying why). Required on the by-hand tier.
    #[serde(default)]
    by_hand: String,
    /// An `#[ignore]`d helper that another test of its file spawns as a child
    /// process (`--ignored --exact <name>`): the parent, which is not gated,
    /// is what runs it, on the parent's tier.
    #[serde(default)]
    child_of: String,
    #[serde(default)]
    note: String,
}

impl TierRow {
    /// A `::*` row covers the gated tests under its prefix that need no
    /// opt-in flag; a test that does is named by a row of its own, whose
    /// configuration names the flag (a run without it is a skip). An exact
    /// row may also name a test that is not gated: a variant, which only a
    /// configuration other than the default makes worth a row (TS.18).
    fn matches(&self, key: &str, test: &DiscoveredTest) -> bool {
        match self.id.strip_suffix("::*") {
            Some(prefix) => {
                test.backend_gated()
                    && test.opt_in_flags.is_empty()
                    && key
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with("::"))
            }
            None => self.id == key,
        }
    }
}

impl DiscoveredTest {
    /// The manifest's key (see [`TierRow::id`]).
    fn tier_key(&self, root: &Path) -> String {
        match self.integration_target(root) {
            Some((package, _, Some(module))) => format!("{package}/tests::{module}::{}", self.name),
            Some((package, target, None)) => format!("{package}/tests::{target}::{}", self.name),
            None => format!("{}::{}", self.relative, self.name),
        }
    }

    /// Whether this one line of a workflow names the test or invokes its
    /// target (whole) or its group binary's module. A line that only names the
    /// test (a matrix `test:` entry) is not a command; the caller decides
    /// whether its job's command runs ignored tests.
    fn covered_by(&self, root: &Path, line: &str) -> bool {
        if line.contains(&self.name) {
            return true;
        }
        match self.integration_target(root) {
            Some((package, target, None)) => invokes_target(line, package, target, true),
            Some((package, target, Some(module))) => {
                invokes_module(line, package, target, &format!("{module}::{}", self.name))
            }
            None => false,
        }
    }
}

/// Whether a command line runs `#[ignore]`d tests at all: cargo's
/// `--ignored` / `--include-ignored`, nextest's `--run-ignored`.
fn runs_ignored(line: &str) -> bool {
    line.split_whitespace().any(|word| {
        matches!(word, "--ignored" | "--include-ignored") || word.starts_with("--run-ignored")
    })
}

/// One line of a workflow, with the job it sits in.
struct WorkflowLine<'a> {
    /// `None` before `jobs:` and for a comment at the jobs' own indentation
    /// (a note between jobs belongs to none of them).
    job: Option<&'a str>,
    comment: bool,
    text: &'a str,
}

fn workflow_lines(workflow: &str) -> Vec<WorkflowLine<'_>> {
    let mut in_jobs = false;
    let mut job: Option<&str> = None;
    workflow
        .lines()
        .map(|text| {
            let trimmed = text.trim_start();
            let depth = text.len() - trimmed.len();
            let comment = trimmed.starts_with('#');
            if text == "jobs:" {
                in_jobs = true;
                job = None;
            } else if in_jobs && depth == 2 && !comment && trimmed.ends_with(':') {
                job = Some(trimmed.trim_end_matches(':'));
            } else if in_jobs && depth == 0 && !trimmed.is_empty() && !comment {
                in_jobs = false;
                job = None;
            }
            let own = if comment && depth <= 2 { None } else { job };
            WorkflowLine {
                job: own,
                comment,
                text,
            }
        })
        .collect()
}

/// Each job's tier, read from its job-level `if:` (doc 43 TS.2): none runs on
/// every push and pull request; the Sunday cron is weekly; any schedule is
/// nightly; `workflow_dispatch` alone is dispatch.
fn job_tiers(workflow: &str) -> BTreeMap<String, &'static str> {
    let mut tiers = BTreeMap::new();
    for line in workflow_lines(workflow) {
        let Some(job) = line.job else { continue };
        tiers.entry(job.to_string()).or_insert("pr");
        let Some(condition) = line.text.strip_prefix("    if:") else {
            continue;
        };
        let tier = if condition.contains(&format!("github.event.schedule == '{WEEKLY_CRON}'")) {
            "weekly"
        } else if condition.contains("'schedule'") {
            "nightly"
        } else if condition.contains("'workflow_dispatch'") {
            "dispatch"
        } else {
            panic!("ci.yml job `{job}`: cannot read a tier from `if:{condition}`")
        };
        tiers.insert(job.to_string(), tier);
    }
    tiers
}

fn tier_manifest(root: &Path) -> TierManifest {
    let path = root.join("crates/mush-contract-tests").join(TIERS_MANIFEST);
    let text = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("{TIERS_MANIFEST}: {error}"))
}

/// A revision as a `by-hand:` tag spells it: 8 to 40 lowercase hex digits.
fn is_revision(tag: &str) -> bool {
    (8..=40).contains(&tag.len())
        && tag
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// The `NAME` / `NAME=VALUE` pairs of `config` the job does not set: each
/// wants a non-comment line of the job holding `NAME:` or `NAME=` (and, with
/// a value, the value).
fn job_does_not_set(lines: &[WorkflowLine<'_>], job: &str, config: &str) -> Vec<String> {
    config
        .split_whitespace()
        .filter(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            !lines.iter().any(|line| {
                line.job == Some(job)
                    && !line.comment
                    && (line.text.contains(&format!("{name}:"))
                        || line.text.contains(&format!("{name}=")))
                    && line.text.contains(value)
            })
        })
        .map(str::to_owned)
        .collect()
}

/// Whether the script names the test, or its integration target as `--test
/// <target>` (a shell line) or `"--test","<target>"` (an argument list).
fn script_runs(root: &Path, via: &str, test: &DiscoveredTest) -> bool {
    let script = fs::read_to_string(root.join(via)).unwrap_or_default();
    script.contains(&test.name)
        || test.integration_target(root).is_some_and(|(_, target, _)| {
            script.contains(&format!("--test {target}"))
                || script.contains(&format!("\"--test\",\"{target}\""))
        })
}

/// The workflows a row is checked against.
struct Workflows {
    ci: String,
    gate: String,
    tiers: BTreeMap<String, &'static str>,
}

/// The tree's tests, with the opt-ins `test-tiers.toml` declares (`kind =
/// "opt-in"`) counted as opt-in flags: a test that returns early unless one
/// is set is gated like a `RUN_*` one (doc 43 TS.18).
fn tiered_tests(root: &Path, manifest: &TierManifest) -> Vec<DiscoveredTest> {
    let declared: BTreeSet<&str> = manifest
        .knobs
        .iter()
        .filter(|knob| knob.kind == "opt-in")
        .map(|knob| knob.name.as_str())
        .collect();
    let mut tests = discovered_tests(root);
    for test in &mut tests {
        let extra: Vec<String> = test
            .early_return_reads
            .iter()
            .filter(|name| declared.contains(name.as_str()))
            .cloned()
            .collect();
        test.opt_in_flags.extend(extra);
    }
    tests
}

/// Checks one row against one test it matches; the problems, if any.
fn row_problems(
    root: &Path,
    flows: &Workflows,
    row: &TierRow,
    test: &DiscoveredTest,
) -> Vec<String> {
    let mut problems = Vec::new();
    let key = test.tier_key(root);
    for flag in &test.opt_in_flags {
        if !row
            .config
            .split_whitespace()
            .any(|pair| pair.split('=').next() == Some(flag.as_str()))
        {
            problems.push(format!(
                "{key}: opts in by {flag}, which the row's config does not set"
            ));
        }
    }
    if !row.child_of.is_empty() {
        let source =
            fs::read_to_string(root.join("crates").join(&test.relative)).unwrap_or_default();
        if row.tier != "pr" || !row.job.is_empty() {
            problems.push(format!(
                "{key}: a child row is on its parent's tier, pr, with no job"
            ));
        }
        if !source.contains(&format!("fn {}(", row.child_of))
            || source.matches(&test.name).count() < 2
        {
            problems.push(format!(
                "{key}: its file has no test `{}` that names it as a child",
                row.child_of
            ));
        }
        return problems;
    }
    if row.tier == "by-hand" {
        if !row.job.is_empty() || !row.via.is_empty() {
            problems.push(format!("{key}: a by-hand row names no job"));
        }
        if !(is_revision(&row.by_hand) || (row.by_hand == "never" && !row.note.is_empty())) {
            problems.push(format!(
                "{key}: a by-hand row carries `by_hand` = the last run's revision \
                 (or `never` with a note), not {:?}",
                row.by_hand
            ));
        }
        return problems;
    }
    if !row.by_hand.is_empty() && !is_revision(&row.by_hand) {
        problems.push(format!("{key}: `by_hand` is a revision when it is set"));
    }
    // The VPS gate: a dispatch of one kind, whose step runs the `via` script.
    if let Some(kind) = row.job.strip_prefix("gate.yml:") {
        if row.tier != "dispatch" {
            problems.push(format!(
                "{key}: gate.yml is dispatch-only, not {}",
                row.tier
            ));
        }
        if !flows.gate.contains(&format!("inputs.kind == '{kind}'")) {
            problems.push(format!("{key}: gate.yml has no `{kind}` kind"));
        }
        if row.via.is_empty() || !flows.gate.contains(&format!("run: {}", row.via)) {
            problems.push(format!(
                "{key}: gate.yml's `{kind}` step runs no `via` script"
            ));
        }
        if !script_runs(root, &row.via, test) {
            problems.push(format!("{key}: {} does not run the test", row.via));
        }
        return problems;
    }
    let Some(job_tier) = flows.tiers.get(&row.job) else {
        problems.push(format!("{key}: ci.yml has no job `{}`", row.job));
        return problems;
    };
    if *job_tier != row.tier {
        problems.push(format!(
            "{key}: job `{}` is on the {job_tier} tier, the row says {}",
            row.job, row.tier
        ));
    }
    let lines = workflow_lines(&flows.ci);
    let in_job = |line: &&WorkflowLine<'_>| line.job == Some(row.job.as_str()) && !line.comment;
    // An `#[ignore]`d test runs only under a command that asks for ignored
    // tests: on the covering line itself, or — for a matrix entry that only
    // names it — on a command of the same job.
    let job_runs_ignored = lines
        .iter()
        .filter(in_job)
        .any(|line| line.text.contains("cargo ") && runs_ignored(line.text));
    let runs = |line: &WorkflowLine<'_>| {
        !test.ignored
            || runs_ignored(line.text)
            || (!line.text.contains("cargo ") && job_runs_ignored)
    };
    let covered = if row.via.is_empty() {
        lines
            .iter()
            .filter(in_job)
            .any(|line| test.covered_by(root, line.text) && runs(line))
    } else {
        script_runs(root, &row.via, test)
            && lines
                .iter()
                .filter(in_job)
                .any(|line| line.text.contains(&row.via))
    };
    if !covered {
        problems.push(format!(
            "{key}: job `{}` does not run it (a comment is not a run{})",
            row.job,
            if test.ignored {
                "; an ignored test needs --ignored or --run-ignored"
            } else {
                ""
            }
        ));
    }
    let unset = job_does_not_set(&lines, &row.job, &row.config);
    if !unset.is_empty() {
        problems.push(format!(
            "{key}: job `{}` does not set {}",
            row.job,
            unset.join(" ")
        ));
    }
    problems
}

/// **Doc 43 TS.2.** Every gated test has a row in `test-tiers.toml`; every row
/// matches a gated test; and each row is true of the workflows: its job is on
/// the tier it says, runs the test by a command (a comment is not a run; an
/// ignored test needs a command that runs ignored tests), and sets the
/// configuration it names. A test only ci.yml's comments name is `by-hand`,
/// with the last run's revision.
#[test]
fn every_gated_test_declares_its_tier_and_the_workflow_agrees() {
    let root = scan::repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci.yml");
    let gate = fs::read_to_string(root.join(".github/workflows/gate.yml")).expect("gate.yml");
    let flows = Workflows {
        tiers: job_tiers(&ci),
        ci,
        gate,
    };
    let manifest = tier_manifest(&root);
    let tests = tiered_tests(&root, &manifest);
    let gated: Vec<&DiscoveredTest> = tests.iter().filter(|t| t.backend_gated()).collect();

    let mut problems: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    for row in &manifest.rows {
        if !TIERS.contains(&row.tier.as_str()) {
            problems.push(format!(
                "{}: tier {:?} is none of {TIERS:?}",
                row.id, row.tier
            ));
        }
        if !seen.insert((row.id.as_str(), row.config.as_str(), row.tier.as_str())) {
            problems.push(format!(
                "{}: a duplicate row ({} {})",
                row.id, row.tier, row.config
            ));
        }
        let matched: Vec<&DiscoveredTest> = tests
            .iter()
            .filter(|test| row.matches(&test.tier_key(&root), test))
            .collect();
        if matched.is_empty() {
            problems.push(format!("{}: a stale row; it matches no test", row.id));
        }
        for test in matched {
            if !test.backend_gated() && row.config.is_empty() {
                problems.push(format!(
                    "{}: the test is not gated, so it runs in its default configuration                      wherever its binary runs; a row for it names a variant's config",
                    row.id
                ));
            }
            problems.extend(row_problems(&root, &flows, row, test));
        }
    }
    for test in &gated {
        let key = test.tier_key(&root);
        if !manifest.rows.iter().any(|row| row.matches(&key, test)) {
            problems.push(format!(
                "{key} ({}): a gated test with no tier; add a row to {TIERS_MANIFEST}",
                test.relative
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{TIERS_MANIFEST} disagrees with the tree or the workflows (doc 43 TS.2):\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_tier_reader_knows_a_job_s_trigger_and_a_comment_from_a_command() {
    let workflow = "\
on:
  push:
jobs:
  every:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test -p a --test t -- --ignored
  # between jobs: cargo test -p a --test b
  night:
    if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'
  sunday:
    if: github.event_name == 'workflow_dispatch' || github.event.schedule == '17 4 * * 0'
    steps:
      # cargo test -p a --test c -- --ignored
  button:
    if: github.event_name == 'workflow_dispatch'
";
    let tiers = job_tiers(workflow);
    assert_eq!(tiers["every"], "pr");
    assert_eq!(tiers["night"], "nightly");
    assert_eq!(tiers["sunday"], "weekly");
    assert_eq!(tiers["button"], "dispatch");
    let lines = workflow_lines(workflow);
    let between = lines.iter().find(|l| l.text.contains("--test b")).unwrap();
    assert_eq!(between.job, None);
    let inside = lines.iter().find(|l| l.text.contains("--test c")).unwrap();
    assert_eq!(inside.job, Some("sunday"));
    assert!(inside.comment);
    assert!(runs_ignored("cargo nextest run -p a --run-ignored only"));
    assert!(runs_ignored("cargo test -p a -- --include-ignored"));
    assert!(!runs_ignored("cargo nextest run -P ci -p a --test t"));
    assert!(is_revision("99b3a4b4"));
    assert!(!is_revision("99B3A4B4") && !is_revision("abc") && !is_revision("never"));
}

// ---- Doc 43 cut TS.18: variant identities ----

/// The crates whose tests §5.9 censused: the stories that carry switches.
const CENSUSED_TESTS: [&str; 2] = ["crates/tacsim/tests", "crates/mush-bot/tests"];

/// The environment variables a test source reads by name: the argument of
/// `env::var`, `env::var_os`, `knob` or an `env_*` helper, as a literal or a
/// `const NAME: &str = "..."` of the same file. A census, not a parser: a name
/// built at run time is invisible to it.
fn env_names_read(source: &str) -> BTreeSet<String> {
    let is_name = |name: &str| {
        name.len() > 1
            && name.starts_with(|c: char| c.is_ascii_uppercase())
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    };
    let mut consts = BTreeMap::new();
    for line in source.lines() {
        let Some((_, rest)) = line.split_once("const ") else {
            continue;
        };
        let Some((ident, value)) = rest.split_once(": &str = \"") else {
            continue;
        };
        if let Some((value, _)) = value.split_once('"') {
            consts.insert(ident.trim().to_string(), value.to_string());
        }
    }
    let mut names = BTreeSet::new();
    for (at, _) in source.match_indices('(') {
        let before = &source[..at];
        let callee_start = before
            .char_indices()
            .rev()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == ':'))
            .map_or(0, |(i, c)| i + c.len_utf8());
        let callee = &before[callee_start..];
        let last = callee.rsplit("::").next().unwrap_or(callee);
        let reads = callee.ends_with("env::var")
            || callee.ends_with("env::var_os")
            || last == "knob"
            || last.starts_with("env_");
        if !reads {
            continue;
        }
        let argument = source[at + 1..].trim_start();
        let name = if let Some(literal) = argument.strip_prefix('"') {
            literal.split_once('"').map(|(name, _)| name.to_string())
        } else {
            let ident: String = argument
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            consts.get(&ident).cloned()
        };
        if let Some(name) = name.filter(|name| is_name(name)) {
            names.insert(name);
        }
    }
    names
}

/// **Doc 43 TS.18.** Every variable tacsim's and the bot's tests read is a
/// named configuration — in the `config` of a `test-tiers.toml` row, where it
/// has a tier and a run — or a declared knob with a kind saying why it needs
/// no row. A new switch in a story therefore arrives with its row or its
/// reason; a variant nobody selects is no longer a configuration with no test
/// under a name the inventory calls covered (§5.9).
#[test]
fn every_variable_a_story_reads_is_a_named_configuration_or_a_declared_knob() {
    let root = scan::repo_root();
    let manifest = tier_manifest(&root);
    let mut read: BTreeMap<String, String> = BTreeMap::new();
    for dir in CENSUSED_TESTS {
        for file in scan::files(&root.join(dir), &["rs"], &["target"]) {
            let source = fs::read_to_string(&file).unwrap_or_default();
            let relative = file
                .strip_prefix(&root)
                .expect("under the root")
                .to_string_lossy()
                .replace('\\', "/");
            for name in env_names_read(&source) {
                read.entry(name).or_insert_with(|| relative.clone());
            }
        }
    }
    assert!(
        read.len() >= 50 && read.contains_key("FIELD_RAID") && read.contains_key("MINE_RATE"),
        "the census found {} variables: it is broken, not the tree",
        read.len()
    );
    let configured: BTreeSet<&str> = manifest
        .rows
        .iter()
        .flat_map(|row| row.config.split_whitespace())
        .map(|pair| pair.split('=').next().unwrap_or(pair))
        .collect();
    let mut problems = Vec::new();
    let mut knobs = BTreeSet::new();
    for knob in &manifest.knobs {
        if !KNOB_KINDS.iter().any(|(kind, _)| *kind == knob.kind) {
            problems.push(format!(
                "knob {}: kind {:?} is none of {KNOB_KINDS:?}",
                knob.name, knob.kind
            ));
        }
        if knob.note.trim().is_empty() {
            problems.push(format!("knob {}: a knob says what it switches", knob.name));
        }
        if !knobs.insert(knob.name.as_str()) {
            problems.push(format!("knob {}: declared twice", knob.name));
        }
        if !read.contains_key(&knob.name) {
            problems.push(format!(
                "knob {}: a stale knob; no censused test reads it",
                knob.name
            ));
        }
        if knob.kind == "opt-in" && !configured.contains(knob.name.as_str()) {
            problems.push(format!(
                "knob {}: an opt-in no row's config sets runs nowhere",
                knob.name
            ));
        }
    }
    for (name, file) in &read {
        if !configured.contains(name.as_str()) && !knobs.contains(name.as_str()) {
            problems.push(format!(
                "{name} (read in {file}): a configuration with no row and no declared knob; \
                 name it in a row's config (with its tier and last run) or declare it a \
                 [[knob]] in {TIERS_MANIFEST}"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{TIERS_MANIFEST}'s variants and knobs disagree with the tests (doc 43 TS.18):\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_variable_census_reads_literals_constants_and_helpers() {
    let source = r#"
const ROLE: &str = "MY_ROLE";
fn x() {
    let a = std::env::var("FIELD_RAID");
    let b = std::env::var_os(ROLE);
    let c = env_u64("TACSIM_SEED", 1);
    let d = knob("GC12_ARENA_S", 120);
    let e = std::env::var(name);
    let f = other("NOT_READ");
    fn env_u32(name: &str) {}
}
"#;
    let names: Vec<String> = env_names_read(source).into_iter().collect();
    assert_eq!(
        names,
        ["FIELD_RAID", "GC12_ARENA_S", "MY_ROLE", "TACSIM_SEED"]
    );
}
