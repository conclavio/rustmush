//! **The process-fault gate** — a workspace policy gate (WS01-S11D).
//!
//! `crates/mush-server/src/process_fault.rs` reads three environment variables
//! and then calls `std::process::abort()` at five exact durability cuts: the
//! four in `persist.rs` that bracket intent, ownership, finalization and the
//! final ack, and the one in `quarantine.rs` between the temp file's `fsync`
//! and its publication. It is the K8 crash matrix's instrument, and it is a
//! remote-controlled durability wrecking bar in anything else.
//!
//! It used to be fenced out of production by `cfg(debug_assertions)`. That is
//! a fence only for as long as nobody sets `debug-assertions = true` in
//! `[profile.release]` — a one-line change with an entirely unrelated
//! motivation (the panic-surface work wants the workspace's debug assertions
//! live) that would silently compile the abort switch into the D1 VPS binary.
//! The fence is now a cargo feature, `mush-server/fault-injection`, off by
//! default and turned on by exactly one `[dev-dependencies]` edge.
//!
//! The scans below are separated from the assertions that use them so each can
//! be run against a fixture: a gate that has never been shown to trip is not
//! yet a gate.

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

/// Every `.rs` file under `crates/*/src`, as (repo-relative path, contents).
fn crate_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let crates = root.join("crates");
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = fs::read_dir(&crates)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", crates.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("src"))
        .filter(|src| src.is_dir())
        .collect();
    assert!(!stack.is_empty(), "no crates/*/src directories found");
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("{} is unreadable: {err}", dir.display()))
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = code_only(
                    &fs::read_to_string(&path)
                        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display())),
                );
                let name = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, text));
            }
        }
    }
    out.sort();
    out
}

/// One source file with its whole-line comments removed. A policy scan reads
/// code, not prose: the doc comment on `process_fault.rs` names both
/// `cfg(debug_assertions)` and the environment variables precisely because it
/// explains why the two are no longer wired together.
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether one source file ends a process — the two calls that never return
/// and take the whole world task with them.
fn terminates_the_process(text: &str) -> bool {
    text.contains("process::abort(") || text.contains("process::exit(")
}

/// Whether one source file gates anything on `cfg(debug_assertions)`.
fn gates_on_debug_assertions(text: &str) -> bool {
    text.contains("cfg(debug_assertions)")
        || text.contains("cfg(not(debug_assertions))")
        || text.contains("cfg_attr(debug_assertions")
}

/// The gate: a file that can end the process must not decide whether it can on
/// `debug_assertions`, because a release profile may switch that on.
fn terminating_path_gated_on_debug_assertions(text: &str) -> bool {
    terminates_the_process(text) && gates_on_debug_assertions(text)
}

#[test]
fn no_crate_source_gates_a_process_terminating_path_on_debug_assertions() {
    let offenders: Vec<String> = crate_sources()
        .into_iter()
        .filter(|(_, text)| terminating_path_gated_on_debug_assertions(text))
        .map(|(name, _)| name)
        .collect();

    assert!(
        offenders.is_empty(),
        "these files decide on `cfg(debug_assertions)` whether they abort or exit the \
         process: {offenders:?}. `debug_assertions` is not a production fence — \
         `[profile.release]` can turn it on for reasons that have nothing to do with \
         process termination. Gate the path on a cargo feature that is off by default \
         and enabled only by the test edge that proves it (see \
         `mush-server/fault-injection`)."
    );
}

#[test]
fn the_termination_scan_trips_on_a_file_that_deserves_it() {
    // The exact shape the gate above exists to catch — process_fault.rs as it
    // stood before WS01-S11D.
    let before = "#[cfg(debug_assertions)]\nfn arm() {}\nfn cut() { std::process::abort(); }\n";
    assert!(terminating_path_gated_on_debug_assertions(before));

    // A feature-gated abort is fine, and so is a debug assertion that cannot
    // end the process.
    let after = "#[cfg(feature = \"fault-injection\")]\nfn cut() { std::process::abort(); }\n";
    assert!(!terminating_path_gated_on_debug_assertions(after));
    let assertion = "#[cfg(debug_assertions)]\nfn check() { debug_assert!(true); }\n";
    assert!(!terminating_path_gated_on_debug_assertions(assertion));

    // Prose is not code. A file that only *explains* the old gate is clean,
    // and a file that hides one under a doc comment is still caught.
    let prose =
        "//! once `cfg(debug_assertions)`, now a feature\nfn cut() { std::process::abort(); }\n";
    assert!(!terminating_path_gated_on_debug_assertions(&code_only(
        prose
    )));
    assert!(terminating_path_gated_on_debug_assertions(&code_only(
        before
    )));
}

#[test]
fn the_crash_injection_seam_is_fenced_by_its_own_feature() {
    let seam: Vec<(String, String)> = crate_sources()
        .into_iter()
        .filter(|(_, text)| text.contains("RUSTMUSH_K8_PROCESS_CUT"))
        .collect();

    assert_eq!(
        seam.len(),
        1,
        "the K8 process-cut environment is read in {} crate sources; it belongs in \
         `crates/mush-server/src/process_fault.rs` alone, where one feature fences it",
        seam.len()
    );
    let (name, text) = &seam[0];
    assert_eq!(name, "crates/mush-server/src/process_fault.rs");
    assert!(
        text.contains("cfg(feature = \"fault-injection\")"),
        "{name} no longer fences the crash-injection hooks behind the \
         `fault-injection` feature"
    );
    assert!(
        !gates_on_debug_assertions(text),
        "{name} is back on `cfg(debug_assertions)`, which a release profile can turn on"
    );
}

#[test]
fn the_feature_is_off_by_default_and_on_only_for_the_kill_proofs() {
    let root = repo_root();

    let server = manifest(&root.join("crates/mush-server/Cargo.toml"));
    let features = server
        .get("features")
        .and_then(toml::Value::as_table)
        .expect("mush-server declares a [features] table");
    assert!(
        features.contains_key("fault-injection"),
        "mush-server no longer declares the `fault-injection` feature; the crash-injection \
         hooks have nothing fencing them"
    );
    let default: Vec<String> = features
        .get("default")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !default.iter().any(|f| f == "fault-injection"),
        "`fault-injection` is in mush-server's default features, so every build has the \
         abort switch"
    );

    // The seam moved rather than disappeared: the split-process kill proofs
    // still turn it on, and they do it from a dev-only edge.
    let gateway = manifest(&root.join("crates/mush-gateway/Cargo.toml"));
    assert!(
        activates_fault_injection(&gateway, "dev-dependencies"),
        "no `[dev-dependencies]` edge activates `mush-server/fault-injection`, so \
         `crates/mush-gateway/tests/authoritative_process_kill.rs` can no longer stop the \
         server at a durability cut and the K8 kill matrix proves nothing"
    );

    // ...and no production edge anywhere does.
    for entry in fs::read_dir(root.join("crates")).expect("crates/ is readable") {
        let path = entry.expect("readable dir entry").path().join("Cargo.toml");
        if !path.is_file() {
            continue;
        }
        let table = manifest(&path);
        for section in ["dependencies", "build-dependencies"] {
            assert!(
                !activates_fault_injection(&table, section),
                "{} activates `mush-server/fault-injection` from [{section}], which puts \
                 the crash-injection hooks in a shipped binary",
                path.display()
            );
        }
    }
}

/// One parsed manifest.
fn manifest(path: &Path) -> toml::Table {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    toml::from_str(&text).unwrap_or_else(|err| panic!("{} does not parse: {err}", path.display()))
}

/// Whether one manifest section turns on `mush-server`'s `fault-injection`.
fn activates_fault_injection(table: &toml::Table, section: &str) -> bool {
    table
        .get(section)
        .and_then(toml::Value::as_table)
        .and_then(|deps| deps.get("mush-server"))
        .and_then(toml::Value::as_table)
        .and_then(|dep| dep.get("features"))
        .and_then(toml::Value::as_array)
        .is_some_and(|features| {
            features
                .iter()
                .any(|f| f.as_str() == Some("fault-injection"))
        })
}
