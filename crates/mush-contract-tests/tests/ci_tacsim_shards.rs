//! **tacsim's integration shards cover every test target exactly once** — a
//! CI policy gate.
//!
//! `.github/workflows/ci.yml` runs tacsim's integration test binaries in several
//! `tacsim-integration-N` shards of the `test` job, each running line N of
//! `.github/tacsim-integration-shards.txt` through `scripts/ci-tacsim-shard.sh`.
//! An explicit list is a list someone must keep: a new `crates/tacsim/tests/*.rs`
//! left off it would build in no shard and report green forever. This gate
//! fails on a target that is missing, listed twice or no longer exists, and on
//! a shard line the workflow's matrix does not run. Since test-strategy cut T.3
//! (2026-10-26) the targets are the four group binaries `shard_1`..`shard_4`
//! (`tests/shard_N.rs`, one `mod` line per former file, now under
//! `tests/shard_N/`) and the standalone `harness` and `cost_lane`: a new test
//! MODULE is a `mod` line in a shard root, never a name here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// Shard number -> its targets, from the manifest's `N: target target ...` lines.
fn manifest(text: &str) -> BTreeMap<u32, Vec<String>> {
    let mut shards = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (number, targets) = line
            .split_once(':')
            .unwrap_or_else(|| panic!("shard line `{line}` is not `N: target ...`"));
        let number: u32 = number
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("shard line `{line}` does not start with a number"));
        let targets: Vec<String> = targets.split_whitespace().map(str::to_owned).collect();
        assert!(!targets.is_empty(), "shard {number} lists no targets");
        assert!(
            shards.insert(number, targets).is_none(),
            "shard {number} is listed twice"
        );
    }
    shards
}

/// tacsim's integration test targets: one per top-level `tests/*.rs` file.
fn tacsim_targets(root: &Path) -> BTreeSet<String> {
    let dir = root.join("crates/tacsim/tests");
    fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("{}: {err}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "rs"))
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .collect()
}

#[test]
fn every_tacsim_integration_target_runs_in_exactly_one_shard() {
    let root = repo_root();
    let path = root.join(".github/tacsim-integration-shards.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let shards = manifest(&text);
    let targets = tacsim_targets(&root);

    let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
    for (number, listed) in &shards {
        for target in listed {
            if let Some(first) = seen.insert(target, *number) {
                panic!("`{target}` is listed in shard {first} and shard {number}");
            }
        }
    }
    let listed: BTreeSet<String> = seen.keys().map(|t| (*t).to_owned()).collect();
    let missing: Vec<&String> = targets.difference(&listed).collect();
    let unknown: Vec<&String> = listed.difference(&targets).collect();
    assert!(
        missing.is_empty(),
        "crates/tacsim/tests/ has targets no shard runs; add them to {}: {missing:?}",
        path.display()
    );
    assert!(
        unknown.is_empty(),
        "{} lists targets crates/tacsim/tests/ no longer has: {unknown:?}",
        path.display()
    );
}

#[test]
fn the_workflow_runs_every_listed_shard_and_no_other() {
    let root = repo_root();
    let manifest_path = root.join(".github/tacsim-integration-shards.txt");
    let shards = manifest(
        &fs::read_to_string(&manifest_path)
            .unwrap_or_else(|err| panic!("{}: {err}", manifest_path.display())),
    );
    let workflow_path = root.join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|err| panic!("{}: {err}", workflow_path.display()));

    let numbers: Vec<u32> = shards.keys().copied().collect();
    let expected: Vec<u32> =
        (1..=u32::try_from(shards.len()).expect("a handful of shards")).collect();
    assert_eq!(numbers, expected, "shards are numbered 1..=N with no gaps");

    let in_matrix: BTreeSet<u32> = workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- tacsim-integration-"))
        .filter_map(|rest| rest.trim().parse().ok())
        .collect();
    assert_eq!(
        in_matrix,
        shards.keys().copied().collect::<BTreeSet<u32>>(),
        "{}'s `test` matrix must run exactly the shards {} lists",
        workflow_path.display(),
        manifest_path.display()
    );
    assert!(
        workflow.contains("scripts/ci-tacsim-shard.sh"),
        "{} no longer runs the shards through scripts/ci-tacsim-shard.sh",
        workflow_path.display()
    );
}
