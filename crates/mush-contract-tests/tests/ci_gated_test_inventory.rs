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
//!   (`clean_shutdown_drains_and_snapshots` in `mushd/tests/durability_e2e.rs`
//!   was the case: `redis-faults` invoked its target only with its kill -9
//!   twin's name as the filter, so its Redis branch ran in no job at all.)
//!   A unit test under `src/` must be named verbatim: `-p <pkg> --lib` with a
//!   filter proves nothing about a *different* gated test in the same crate.
//!   A `cargo nextest run` line reads the same way (test-strategy cut T.1):
//!   its filterset (`-E 'test(=name)'`, `--filterset`) is a filter, and its
//!   own value-taking options (`--build-jobs 2`, `--run-ignored only`,
//!   `-P ci`) are not.
//!
//! Tests whose names merely end in `_gated` in the English sense
//! (`chown_is_power_gated`, `harm_is_staff_gated`) are not backend-gated: they
//! run, and fail, under the ordinary `cargo test --workspace` job. The suffix
//! is not the contract — skippability is.
//!
//! Macro-generated tests (`mush-store`'s `redis_gated!` suite) are invisible to
//! a source scan by construction; their target is named by `redis-faults`
//! directly and the second test below pins the census so the scanner cannot
//! quietly stop finding anything.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The exact source form of a live-backend gate, built at runtime so that this
/// file does not match its own needle.
fn redis_gate_needle() -> String {
    format!("env::var({:?})", "REDIS_URL")
}

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// One discovered test function.
#[derive(Debug)]
struct DiscoveredTest {
    name: String,
    /// Path relative to `crates/`, e.g. `mushd/tests/persistence.rs`.
    relative: String,
    ignored: bool,
    /// The test's own body reads the live-backend variable.
    reads_live_backend: bool,
    /// Explicit opt-in flags in an early-returning test body.
    opt_in_flags: BTreeSet<String>,
}

impl DiscoveredTest {
    fn backend_gated(&self) -> bool {
        self.ignored || self.reads_live_backend || !self.opt_in_flags.is_empty()
    }

    /// `Some((package, target))` for `crates/<pkg>/tests/<target>.rs`.
    fn integration_target(&self) -> Option<(&str, &str)> {
        let (package, rest) = self.relative.split_once('/')?;
        let target = rest.strip_prefix("tests/")?.strip_suffix(".rs")?;
        // Only a top-level `tests/<target>.rs` is its own cargo target.
        if target.contains('/') {
            return None;
        }
        Some((package, target))
    }
}

/// Opt-in spelling is part of this source gate's contract. Knobs such as a
/// duration or report path do not make a test optional.
fn opt_in_flags(body: &str) -> BTreeSet<String> {
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
        .filter(|name| name.starts_with("RUN_") || name.ends_with("_RUN"))
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
        "mushd/tests/npc_smoke.rs",
        "live_npc_dialogue_openrouter_smoke",
        "RUN_LLM_SMOKE",
        "Manual live-provider smoke; needs an operator-funded API key.",
    ),
    (
        "mushd/tests/worldgen_smoke.rs",
        "live_worldgen_openrouter_smoke",
        "RUN_LLM_SMOKE",
        "Manual live-provider smoke; needs an operator-funded API key.",
    ),
];

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

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
        });
        index = end + 1;
    }
}

fn discovered_tests(root: &Path) -> Vec<DiscoveredTest> {
    let crates_dir = root.join("crates");
    let mut files = Vec::new();
    collect_rust_files(&crates_dir, &mut files);
    files.sort();
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

#[test]
fn every_backend_gated_proof_is_named_by_a_ci_job() {
    let root = repo_root();
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
        // step covers only the test it names, by its name (checked above).
        if let Some((package, target)) = test.integration_target()
            && invokes_target(&workflow, package, target, true)
        {
            continue;
        }
        unowned.insert(format!("{} ({})", test.name, test.relative));
    }

    assert!(
        unowned.is_empty(),
        "these proofs skip themselves without a live backend and no CI job runs them — \
         name each one (or its `-p <pkg> --test <target>`, with no test-name filter) \
         in .github/workflows/ci.yml:\n  {}",
        unowned.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

#[test]
fn paid_opt_in_exemptions_are_live_and_the_cost_report_has_a_mock_ci_run() {
    let root = repo_root();
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
    let root = repo_root();
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
//   * **L4** cited `cargo test -p mush-contract-tests --test schedule_source`
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
    let root = repo_root();
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
            invocation: "-p mush-server --test production_action_migrations",
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
            invocation: "-p mush-contract-tests --test gate5_fault_matrix",
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

    // §14.3 criterion 4: the lane proofs are named by the *K8 kernel contracts*
    // job, so a reviewer running CI runs them.
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("the CI workflow is part of the tree this policy governs");
    for needle in [
        "-p mush-industry --test e7c4",
        "-p mush-contract-tests --test gate5_fault_matrix",
        "-p mush-server --test production_action_migrations",
    ] {
        assert!(
            workflow.contains(needle),
            "10b §14.3 criterion 4: `cargo test {needle}` must be named by a CI job"
        );
    }
}

#[test]
fn vf_g6_every_proof_10b_section_14_names_exists_in_the_file_it_is_attributed_to() {
    let root = repo_root();
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
            "mush-contract-tests/tests/gate5_fault_matrix.rs",
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
    let matrix =
        fs::read_to_string(root.join("crates/mush-contract-tests/tests/gate5_fault_matrix.rs"))
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
    let harness =
        fs::read_to_string(root.join("crates/mush-contract-tests/tests/scale_harness.rs"))
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
