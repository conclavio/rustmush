//! **tacsim's integration tests run in exactly one place each** — a CI policy
//! gate.
//!
//! Since doc 43 cut TS.12's adoption (D-43-5, 2026-11-04) tacsim's tests are four
//! DOMAIN binaries, `crates/tacsim/tests/<domain>.rs`, with their modules under
//! `tests/<domain>/`, and the standalone `harness` and `cost_lane`.
//!
//! `.github/workflows/ci.yml`'s `tacsim-integration-N` cells run them through
//! `scripts/ci-tacsim-shard.sh N`, as `.github/tacsim-integration-shards.txt`
//! describes:
//! - the `partition:` line names the domain binaries every cell builds, each cell
//!   running its `--partition hash:N/<cells>` share of them;
//! - the numbered line `N:` names the whole targets cell N runs first (harness on
//!   1, cost_lane on 2).
//!
//! An explicit list is a list someone must keep: a new `crates/tacsim/tests/*.rs`
//! left off it would build in no cell and report green forever. This gate fails
//! on any of these:
//! - a target that is missing, listed twice or no longer exists;
//! - a cell the workflow's matrix does not run;
//! - a file under a domain directory that its root's module tree does not
//!   declare (it would compile nowhere);
//! - a root that declares a module outside its own directory or `support/`;
//! - any other directory under `tests/`, such as a `shard_N/` left by a branch
//!   cut before the adoption;
//! - a module name in a binary that ends with another's (cargo's substring filter
//!   would run both).
//!
//! A new test MODULE is a `mod` line in its domain root, never a name here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use mush_contract_tests::scan;

/// The manifest: the partitioned binaries, and cell number -> the whole targets it runs first.
struct Manifest {
    partition: Vec<String>,
    cells: BTreeMap<u32, Vec<String>>,
}

fn manifest(text: &str) -> Manifest {
    let mut partition = None;
    let mut cells = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, targets) = line
            .split_once(':')
            .unwrap_or_else(|| panic!("manifest line `{line}` is not `key: target ...`"));
        let targets: Vec<String> = targets.split_whitespace().map(str::to_owned).collect();
        if key.trim() == "partition" {
            assert!(!targets.is_empty(), "the partition: line names no binaries");
            assert!(
                partition.replace(targets).is_none(),
                "the partition: line is listed twice"
            );
            continue;
        }
        let number: u32 = key
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("manifest line `{line}` is neither `partition:` nor `N:`"));
        assert!(
            cells.insert(number, targets).is_none(),
            "cell {number} is listed twice"
        );
    }
    Manifest {
        partition: partition.expect("the manifest has a partition: line"),
        cells,
    }
}

fn read_manifest(root: &Path) -> (PathBuf, Manifest) {
    let path = root.join(".github/tacsim-integration-shards.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    (path, manifest(&text))
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
fn every_tacsim_integration_target_runs_exactly_once() {
    let root = scan::repo_root();
    let (path, manifest) = read_manifest(&root);
    let targets = tacsim_targets(&root);

    let mut seen: BTreeMap<&str, String> = BTreeMap::new();
    for target in &manifest.partition {
        if let Some(first) = seen.insert(target, "the partition".to_owned()) {
            panic!("`{target}` is listed in {first} and in the partition");
        }
    }
    for (number, listed) in &manifest.cells {
        for target in listed {
            if let Some(first) = seen.insert(target, format!("cell {number}")) {
                panic!("`{target}` is listed in {first} and in cell {number}");
            }
        }
    }
    let listed: BTreeSet<String> = seen.keys().map(|t| (*t).to_owned()).collect();
    let missing: Vec<&String> = targets.difference(&listed).collect();
    let unknown: Vec<&String> = listed.difference(&targets).collect();
    assert!(
        missing.is_empty(),
        "crates/tacsim/tests/ has targets no cell runs; a test module belongs in a domain root \
         (`#[path = \"<domain>/<file>.rs\"] mod <file>;`), a new standalone target in {}: \
         {missing:?}",
        path.display()
    );
    assert!(
        unknown.is_empty(),
        "{} lists targets crates/tacsim/tests/ no longer has: {unknown:?}",
        path.display()
    );
}

#[test]
fn the_workflow_runs_every_listed_cell_and_no_other() {
    let root = scan::repo_root();
    let (manifest_path, manifest) = read_manifest(&root);
    let workflow_path = root.join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|err| panic!("{}: {err}", workflow_path.display()));

    let numbers: Vec<u32> = manifest.cells.keys().copied().collect();
    let expected: Vec<u32> =
        (1..=u32::try_from(manifest.cells.len()).expect("a handful of cells")).collect();
    assert_eq!(numbers, expected, "cells are numbered 1..=N with no gaps");

    let in_matrix: BTreeSet<u32> = workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- tacsim-integration-"))
        .filter_map(|rest| rest.trim().parse().ok())
        .collect();
    assert_eq!(
        in_matrix,
        manifest.cells.keys().copied().collect::<BTreeSet<u32>>(),
        "{}'s `test` matrix must run exactly the cells {} lists",
        workflow_path.display(),
        manifest_path.display()
    );
    assert!(
        workflow.contains("scripts/ci-tacsim-shard.sh"),
        "{} no longer runs the cells through scripts/ci-tacsim-shard.sh",
        workflow_path.display()
    );
    let script_path = root.join("scripts/ci-tacsim-shard.sh");
    let script = fs::read_to_string(&script_path)
        .unwrap_or_else(|err| panic!("{}: {err}", script_path.display()));
    assert!(
        script.contains("--partition \"hash:"),
        "{} no longer runs a hash partition of the domain binaries",
        script_path.display()
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

/// The domain roots `tests/<domain>.rs` (the manifest's `partition:` line), by target name.
fn domain_roots(root: &Path) -> Vec<(String, PathBuf)> {
    let tests = root.join("crates/tacsim/tests");
    let (_, manifest) = read_manifest(root);
    manifest
        .partition
        .into_iter()
        .map(|target| {
            let path = tests.join(format!("{target}.rs"));
            (target, path)
        })
        .collect()
}

/// Every file a root's module tree reaches, the root included.
fn reached_from(root_file: &Path) -> BTreeSet<PathBuf> {
    let mut reached: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: Vec<(PathBuf, bool)> = vec![(normalise(root_file), true)];
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
    reached
}

/// Doc 43 §5.3 (cut TS.0), for the domain layout: a file under `tests/<domain>/`
/// that its root's module tree does not declare compiles in no binary and fails
/// nothing, so the target guard above cannot see it. Every such file must be
/// reached from that domain's root by its `mod` declarations (with `#[path]`
/// siblings and subdirectories), and a root declares modules only from its own
/// directory and `support/`.
#[test]
fn every_file_under_a_domain_directory_is_declared_by_its_root() {
    let root = scan::repo_root();
    let tests = normalise(&root.join("crates/tacsim/tests"));
    let support = tests.join("support");
    for (domain, root_file) in domain_roots(&root) {
        let dir = tests.join(&domain);
        let root_file = normalise(&root_file);
        let reached = reached_from(&root_file);
        let foreign: Vec<&PathBuf> = reached
            .iter()
            .filter(|file| **file != root_file)
            .filter(|file| !file.starts_with(&dir) && !file.starts_with(&support))
            .collect();
        assert!(
            foreign.is_empty(),
            "{domain}.rs declares modules outside tests/{domain}/ and tests/support/; a \
             domain's modules live in its own directory: {foreign:?}"
        );
        let under: BTreeSet<PathBuf> = scan::rust_files(&dir)
            .iter()
            .map(|file| normalise(file))
            .collect();
        let orphans: Vec<&PathBuf> = under.difference(&reached).collect();
        assert!(
            orphans.is_empty(),
            "these files are under tests/{domain}/ but {domain}.rs does not declare them, so \
             they compile nowhere; add a `#[path = \"{domain}/<file>.rs\"] mod <file>;` line to \
             {domain}.rs or delete them: {orphans:?}"
        );
    }
}

/// The adoption retired the shard layout: a directory under `tests/` is a
/// domain's, `support/` or `harness_baseline/`. A `shard_N/` (or any other)
/// directory is a branch cut before 2026-11-04 merged without moving its modules
/// into their domain.
#[test]
fn tests_holds_only_domain_support_and_baseline_directories() {
    let root = scan::repo_root();
    let tests = root.join("crates/tacsim/tests");
    let mut allowed: BTreeSet<String> = domain_roots(&root).into_iter().map(|(d, _)| d).collect();
    allowed.insert("support".to_owned());
    allowed.insert("harness_baseline".to_owned());
    let strays: Vec<String> = fs::read_dir(&tests)
        .unwrap_or_else(|err| panic!("{}: {err}", tests.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|name| !allowed.contains(name))
        .collect();
    assert!(
        strays.is_empty(),
        "crates/tacsim/tests/ has directories that are not a domain's: {strays:?}. Since \
         D-43-5 (2026-11-04) a test module lives at tests/<domain>/<module>.rs with a `mod` \
         line in tests/<domain>.rs; move the files there"
    );
}

/// Doc 43 §5.3 (cut TS.0): one module of a domain binary runs with `cargo test
/// --test <domain> <module>::`, a substring filter, so a module whose name ends
/// with another's in the same binary (`gc5` and `field_gc5`) drags the other's
/// tests in.
#[test]
fn no_module_name_in_a_domain_ends_with_another_in_the_same_domain() {
    for (domain, path) in domain_roots(&scan::repo_root()) {
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
                    "{domain}: module `{long}` ends with module `{short}`, so the filter \
                     `{short}::` also runs `{long}`'s tests; rename one"
                );
            }
        }
    }
}
