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
//!   environment variable, or the test carries `#[ignore]` — the two ways a
//!   proof can skip itself under `cargo test --workspace`;
//! - it is **covered** when `ci.yml` names the function verbatim, or (for an
//!   integration target) names that whole target as `-p <pkg> --test <target>`.
//!   A unit test under `src/` must be named verbatim: `-p <pkg> --lib` with a
//!   filter proves nothing about a *different* ignored test in the same crate.
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
}

impl DiscoveredTest {
    const fn backend_gated(&self) -> bool {
        self.ignored || self.reads_live_backend
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
        if workflow.contains(&test.name) {
            continue;
        }
        if let Some((package, target)) = test.integration_target()
            && workflow.contains(&format!("-p {package} --test {target}"))
        {
            continue;
        }
        unowned.insert(format!("{} ({})", test.name, test.relative));
    }

    assert!(
        unowned.is_empty(),
        "these proofs skip themselves without a live backend and no CI job runs them — \
         name each one (or its `-p <pkg> --test <target>`) in .github/workflows/ci.yml:\n  {}",
        unowned.into_iter().collect::<Vec<_>>().join("\n  ")
    );
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
    // +2 for H3's `scale_world_boot_redis_gated` and `scale_authoritative_soak_redis_gated`.
    assert_eq!(
        live.len(),
        13,
        "the live-backend `*_gated` census changed: {live:?}"
    );
    for name in &live {
        assert!(
            name.contains("redis"),
            "a live-backend proof must say which backend it needs: {name}"
        );
    }
}
