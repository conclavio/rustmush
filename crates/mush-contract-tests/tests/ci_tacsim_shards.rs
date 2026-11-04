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
//! (2026-10-26) the targets are the group binaries `shard_1`..`shard_10` (four until 2026-10-27, six until 2026-11-03)
//! (`tests/shard_N.rs`, one `mod` line per former file, now under
//! `tests/shard_N/`) and the standalone `harness` and `cost_lane`: a new test
//! MODULE is a `mod` line in a shard root, never a name here. Since doc 43 cut
//! TS.0 it also fails on a file under `tests/shard_N/` that no root's module
//! tree declares (it would compile nowhere), and on a module name in a shard
//! that ends with another's (cargo's substring filter would run both).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use mush_contract_tests::scan;

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
    let root = scan::repo_root();
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
    let root = scan::repo_root();
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

/// A source file's `mod` declarations, as `(name, Some(path))` for a
/// `#[path = "..."]`-attributed one and `(name, None)` for a plain `mod name;`.
/// Comment lines are skipped (a doc comment may quote a declaration), and so is
/// an inline `mod name {`, whose children live in the same file.
fn mod_declarations(source: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut pending_path: Option<String> = None;
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#[path") {
            pending_path = rest.split('"').nth(1).map(str::to_owned);
            continue;
        }
        if line.starts_with("#[") {
            continue;
        }
        let decl = line
            .strip_prefix("pub(crate) ")
            .or_else(|| line.strip_prefix("pub(super) "))
            .or_else(|| line.strip_prefix("pub "))
            .unwrap_or(line);
        if let Some(name) = decl
            .strip_prefix("mod ")
            .and_then(|rest| rest.strip_suffix(';'))
        {
            out.push((name.trim().to_owned(), pending_path.take()));
        } else {
            pending_path = None;
        }
    }
    out
}

/// The file a declaration in `file` loads: a `#[path]` is relative to the
/// declaring file's directory; a plain `mod name;` in a crate root or a
/// `mod.rs` is `name.rs` (or `name/mod.rs`) beside it, and anywhere else it is
/// under the directory named for the declaring file.
fn declared_file(file: &Path, is_root: bool, name: &str, path: Option<&str>) -> PathBuf {
    let dir = file.parent().expect("a source file has a directory");
    if let Some(path) = path {
        return dir.join(path);
    }
    let base = if is_root || file.file_name().is_some_and(|n| n == "mod.rs") {
        dir.to_path_buf()
    } else {
        dir.join(file.file_stem().expect("a source file has a stem"))
    };
    let flat = base.join(format!("{name}.rs"));
    if flat.is_file() {
        flat
    } else {
        base.join(name).join("mod.rs")
    }
}

/// `.` and `..` folded out, so two spellings of one file compare equal.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The shard roots `tests/shard_N.rs`, by target name, in name order.
fn shard_roots(root: &Path) -> Vec<(String, PathBuf)> {
    let tests = root.join("crates/tacsim/tests");
    tacsim_targets(root)
        .into_iter()
        .filter(|target| target.starts_with("shard_"))
        .map(|target| {
            let path = tests.join(format!("{target}.rs"));
            (target, path)
        })
        .collect()
}

/// Doc 43 §5.3 (cut TS.0): a file under `tests/shard_N/` that no root's module
/// tree declares compiles in no binary and fails nothing, so the target guard
/// above cannot see it. Every such file must be reached from some shard root
/// by its `mod` declarations (with `#[path]` siblings and subdirectories).
#[test]
fn every_file_under_a_shard_directory_is_declared_by_a_root() {
    let root = scan::repo_root();
    let tests = root.join("crates/tacsim/tests");
    let mut reached: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: Vec<(PathBuf, bool)> = shard_roots(&root)
        .into_iter()
        .map(|(_, path)| (normalise(&path), true))
        .collect();
    while let Some((file, is_root)) = queue.pop() {
        if !reached.insert(file.clone()) {
            continue;
        }
        let source =
            fs::read_to_string(&file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
        for (name, path) in mod_declarations(&source) {
            let child = normalise(&declared_file(&file, is_root, &name, path.as_deref()));
            assert!(
                child.is_file(),
                "{} declares `mod {name}` at {}, which does not exist",
                file.display(),
                child.display()
            );
            queue.push((child, false));
        }
    }

    let mut under_shards = BTreeSet::new();
    for entry in fs::read_dir(&tests).unwrap_or_else(|err| panic!("{}: {err}", tests.display())) {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("shard_"))
        {
            under_shards.extend(scan::rust_files(&path).iter().map(|file| normalise(file)));
        }
    }
    let orphans: Vec<&PathBuf> = under_shards.difference(&reached).collect();
    assert!(
        orphans.is_empty(),
        "these files are under a shard directory but no shard root declares them, so they \
         compile nowhere; add a `#[path = \"shard_N/<file>.rs\"] mod <file>;` line to a root \
         (the shard with the most room) or delete them: {orphans:?}"
    );
}

/// Doc 43 §5.3 (cut TS.0): one module of a group binary runs with `cargo test
/// --test shard_N <module>::`, a substring filter, so a module whose name ends
/// with another's in the same binary (`gc5` and `field_gc5`) drags the other's
/// tests in. The merge script refused this; the guard now does.
#[test]
fn no_module_name_in_a_shard_ends_with_another_in_the_same_shard() {
    for (shard, path) in shard_roots(&scan::repo_root()) {
        let source =
            fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        let names: BTreeSet<String> = mod_declarations(&source)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        for long in &names {
            for short in &names {
                assert!(
                    long == short || !long.ends_with(short.as_str()),
                    "{shard}: module `{long}` ends with module `{short}`, so the filter \
                     `{short}::` also runs `{long}`'s tests; rename one"
                );
            }
        }
    }
}
