//! **The toggleable-feature inventory** — a workspace policy gate (WS24-S4).
//!
//! `cargo clippy --workspace --all-targets` and `cargo build --workspace
//! --all-targets` compile every crate exactly once, with its *default* feature
//! set. A cargo feature that is off by default is therefore code that CI never
//! compiles: it rots silently and the first thing that notices is a deploy.
//!
//! That is not hypothetical here. `mush-net-web`'s `tls` feature carries the
//! whole native-HTTPS path (`serve_tls`, the rustls provider install, the PEM
//! loader), `mush-gateway/tls` and `mushd/tls` forward to it, and
//! `mush-gateway --features tls` is a **production** build — the proxyless
//! split deploy. Before this gate, zero jobs in `.github/workflows/ci.yml`
//! mentioned `tls` at all.
//!
//! So: read every `crates/*/Cargo.toml` `[features]` table and every
//! `--features` invocation in `ci.yml` as data, and assert that each
//! **toggleable** feature — every entry in a `[features]` table other than the
//! `default` key itself — is either named by a job that builds the owning
//! package, or carries an explicit allowlist entry that says why not. An
//! allowlist entry is a decision on the record; a feature nobody compiles and
//! nobody mentions is an accident.
//!
//! The census below is pinned exactly, so this file fails in both directions:
//! a **new** feature with no job breaks it, and a **deleted** job breaks it
//! too. A parser that quietly stops finding anything cannot pass vacuously.

use std::collections::BTreeSet;
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

/// One `<package>/<feature>` pair, the way cargo spells it on a command line
/// with more than one `-p` selected.
fn pair(package: &str, feature: &str) -> String {
    format!("{package}/{feature}")
}

/// Every toggleable feature in the workspace, as `<package>/<feature>`.
///
/// Deliberately a line scanner, not a TOML parser: the manifests in this tree
/// are hand-written and rustfmt-adjacent, one feature per line, and a scanner
/// keeps this gate free of a toml dependency it would otherwise be the only
/// user of. A feature whose value list wraps across lines would be missed —
/// the pinned census below is what makes that visible rather than silent.
fn declared_features(root: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let crates_dir = root.join("crates");
    let entries = fs::read_dir(&crates_dir).expect("crates/ is readable");
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(source) = fs::read_to_string(&manifest) else {
            continue;
        };
        let package = entry.file_name().to_string_lossy().into_owned();
        let mut in_features = false;
        for line in source.lines() {
            let code = line.trim();
            if code.starts_with('[') {
                in_features = code == "[features]";
                continue;
            }
            if !in_features || code.is_empty() || code.starts_with('#') {
                continue;
            }
            let Some((name, _)) = code.split_once('=') else {
                continue;
            };
            let name = name.trim();
            // `default` is not toggleable: it is what every job already
            // builds. Its *members* are, through `--no-default-features`.
            if name.is_empty() || name == "default" {
                continue;
            }
            found.insert(pair(&package, name));
        }
    }
    found
}

/// Every `<package>/<feature>` pair that `.github/workflows/ci.yml` actually
/// compiles, resolved the way cargo resolves it.
///
/// On one command line, a bare `--features a,b` applies to whichever of the
/// `-p`-selected packages actually have a feature of that name, while
/// `--features pkg/feat` names its package itself and is an error if that
/// package has no such feature. Both forms are resolved here, the same way,
/// so the gate reads the workflow as cargo would rather than as a grep would.
fn ci_built_features(workflow: &str) -> BTreeSet<String> {
    let mut built = BTreeSet::new();
    for line in workflow.lines() {
        if !line.contains("--features") {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let mut packages: Vec<&str> = Vec::new();
        let mut requested: Vec<&str> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            if *token == "-p" || *token == "--package" {
                if let Some(name) = tokens.get(index + 1) {
                    packages.push(name);
                }
            } else if *token == "--features" {
                if let Some(list) = tokens.get(index + 1) {
                    requested.push(list);
                }
            } else if let Some(list) = token.strip_prefix("--features=") {
                requested.push(list);
            }
        }
        for list in requested {
            for feature in list.split(',').map(str::trim).filter(|f| !f.is_empty()) {
                if let Some((package, name)) = feature.split_once('/') {
                    built.insert(pair(package, name));
                } else {
                    for package in &packages {
                        built.insert(pair(package, feature));
                    }
                }
            }
        }
    }
    built
}

/// A toggleable feature that no job compiles on purpose.
struct Allowed {
    package: &'static str,
    feature: &'static str,
    /// Why no job names it. Read by a human at review time; the test only
    /// enforces that one exists.
    reason: &'static str,
    /// The claim is "a `[dev-dependencies]` edge turns it on, so `cargo test
    /// --workspace` compiles it". That claim is checked below against the
    /// manifests rather than trusted.
    dev_dep_activated: bool,
}

/// The allowlist. Adding a line here is the decision to leave a feature
/// uncompiled by any job of its own; it should be as hard to write as it is
/// easy to read.
const ALLOWED: &[Allowed] = &[
    Allowed {
        package: "mush-llm",
        feature: "anthropic",
        reason: "a member of `default`, so every workspace job compiles it; only a \
                 `--no-default-features` subset job would toggle it, and there is none",
        dev_dep_activated: false,
    },
    Allowed {
        package: "mush-llm",
        feature: "openai",
        reason: "a member of `default`, so every workspace job compiles it; only a \
                 `--no-default-features` subset job would toggle it, and there is none",
        dev_dep_activated: false,
    },
    Allowed {
        package: "mush-llm",
        feature: "mock",
        reason: "a member of `default`, so every workspace job compiles it; only a \
                 `--no-default-features` subset job would toggle it, and there is none",
        dev_dep_activated: false,
    },
    Allowed {
        package: "mush-sim",
        feature: "testkit",
        reason: "turned on by `mush-campaign` and `mush-propagation` through a \
                 `[dev-dependencies]` edge, so `cargo test --workspace` unifies it on \
                 and compiles it",
        dev_dep_activated: true,
    },
    Allowed {
        package: "mush-sim",
        feature: "debug-checks",
        reason: "turned on by `mush-server`'s `[dev-dependencies]` edge (WS01-S11D), so \
                 `cargo test --workspace` unifies it on and compiles it",
        dev_dep_activated: true,
    },
    Allowed {
        package: "mush-housing",
        feature: "debug-checks",
        reason: "turned on by `mush-server`'s `[dev-dependencies]` edge (WS01-S11D), so \
                 `cargo test --workspace` unifies it on and compiles it",
        dev_dep_activated: true,
    },
    Allowed {
        package: "mush-server",
        feature: "fault-injection",
        reason: "turned on by `mush-gateway`'s and `mushd`'s `[dev-dependencies]` edges, \
                 so `cargo test --workspace` unifies it on and compiles it",
        dev_dep_activated: true,
    },
];

/// The census, pinned. Both halves matter: a feature that appears without a
/// job breaks the coverage test, and a feature that *disappears* — or a
/// scanner that stops finding one — breaks this list.
const CENSUS: &[&str] = &[
    "mush-gateway/tls",
    "mush-housing/debug-checks",
    "mush-llm/anthropic",
    "mush-llm/mock",
    "mush-llm/openai",
    "mush-net-web/tls",
    "mush-proto/ts",
    "mush-server/fault-injection",
    "mush-sim/debug-checks",
    "mush-sim/testkit",
    "mush-testkit/toy-ruleset",
    "mushd/tls",
];

fn allowlisted() -> BTreeSet<String> {
    ALLOWED
        .iter()
        .map(|entry| pair(entry.package, entry.feature))
        .collect()
}

#[test]
fn every_toggleable_feature_is_built_by_a_ci_job() {
    let root = repo_root();
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("the CI workflow is part of the tree this policy governs");
    let declared = declared_features(&root);
    let built = ci_built_features(&workflow);
    let allowed = allowlisted();

    // The scanner must be finding *something* on both sides, or "no
    // uncovered features" means nothing.
    assert!(
        built.contains("mush-proto/ts"),
        "the ci.yml `--features` scanner found nothing it should have: {built:?}"
    );

    let uncovered: Vec<String> = declared
        .iter()
        .filter(|feature| !built.contains(*feature) && !allowed.contains(*feature))
        .cloned()
        .collect();
    assert!(
        uncovered.is_empty(),
        "these cargo features are off by default and no CI job compiles them — add a \
         `--features` invocation to .github/workflows/ci.yml, or an ALLOWED entry here \
         saying why not:\n  {}",
        uncovered.join("\n  ")
    );

    // A deleted job is the same failure seen from the other side: the three
    // `tls` features have no default build behind them at all.
    for feature in ["mush-net-web/tls", "mush-gateway/tls", "mushd/tls"] {
        assert!(
            built.contains(feature),
            "`{feature}` is compiled by no ci.yml job; `mush-gateway --features tls` is a \
             production build and nothing else in CI touches the native-TLS path"
        );
    }
}

#[test]
fn the_scanner_still_finds_the_known_features() {
    let root = repo_root();
    let declared = declared_features(&root);
    let census: BTreeSet<String> = CENSUS.iter().map(|f| (*f).to_string()).collect();
    assert_eq!(
        declared, census,
        "the toggleable-feature census moved: pin the new set in CENSUS and decide, for \
         each addition, whether a CI job compiles it or an ALLOWED entry excuses it"
    );
    let crates: BTreeSet<&str> = declared
        .iter()
        .filter_map(|f| f.split('/').next())
        .collect();
    assert_eq!(
        (declared.len(), crates.len()),
        (12, 9),
        "11 toggleable features across 8 crates at the WS24-S4 fold, plus mush-testkit/toy-ruleset \n         (16b cut R-b): {declared:?}"
    );
}

#[test]
fn the_allowlist_states_a_reason() {
    let root = repo_root();
    let declared = declared_features(&root);
    for entry in ALLOWED {
        let name = pair(entry.package, entry.feature);
        assert!(
            !entry.reason.trim().is_empty(),
            "`{name}` is allowlisted with no justification"
        );
        assert!(
            declared.contains(&name),
            "`{name}` is allowlisted but no longer exists — drop the ALLOWED entry"
        );
        if entry.dev_dep_activated {
            assert!(
                dev_dependency_activates(&root, entry.package, entry.feature),
                "`{name}` is allowlisted as dev-dep-activated, but no crate's \
                 `[dev-dependencies]` turns it on — the reason is stale and the feature \
                 is compiled by nothing"
            );
        }
    }
}

/// Does some crate's `[dev-dependencies]` (or a target-specific variant) name
/// `<package>` with `<feature>` turned on?
fn dev_dependency_activates(root: &Path, package: &str, feature: &str) -> bool {
    let crates_dir = root.join("crates");
    let Ok(entries) = fs::read_dir(&crates_dir) else {
        return false;
    };
    let needle = format!("\"{feature}\"");
    for entry in entries.flatten() {
        let Ok(source) = fs::read_to_string(entry.path().join("Cargo.toml")) else {
            continue;
        };
        let mut in_dev = false;
        for line in source.lines() {
            let code = line.trim();
            if code.starts_with('[') {
                in_dev = code.ends_with("dev-dependencies]");
                continue;
            }
            if !in_dev || code.starts_with('#') {
                continue;
            }
            let Some((name, rest)) = code.split_once('=') else {
                continue;
            };
            if name.trim() == package && rest.contains("features") && rest.contains(&needle) {
                return true;
            }
        }
    }
    false
}
