//! **The world reads the time through its provider** (doc 43c §2, unit TS.10e).
//!
//! Since TS.10b the world loop reads every time that gates behaviour (its `now`, the
//! timer and outage wakes, `fire_due`, the shutdown grace) through `ServerDeps::time`, a
//! `mush_config::TimeSource`. Production passes `System`, the OS clock; a paused test
//! rig passes `Tokio`, so a `start_paused` runtime moves the world's unix time and its
//! sleeps together. A new direct read of the OS clock in the world task would quietly
//! break that: it would read real time in a paused test.
//!
//! So: no `unix_now_s()`, `unix_now_ms()` or `SystemTime::now()` call in the production
//! code of any `world_task*` or `arena_task*` file under a crate's `src/` (tests aside),
//! except the sites below. Each is listed with its count and its reason, and the count is
//! exact, so a new read fails here and a removed one asks for its row to go. The provider
//! itself (`crates/mush-config/src/time_source.rs`) is outside the scanned paths.
//!
//! The arena (`arena_task*`) reads no wall clock at all: its clock is a
//! `tokio::time::Instant` base, which a paused runtime already moves. It is scanned so
//! that stays true.

use std::collections::BTreeMap;

use mush_contract_tests::scan;

/// The calls this gate counts.
const NEEDLES: [&str; 3] = ["unix_now_s(", "unix_now_ms(", "SystemTime::now("];

/// The reads that stay on the OS clock: `(file, needle, count, why)`.
const ALLOWED: [(&str, &str, usize, &str); 8] = [
    (
        "crates/mush-server/src/world_task.rs",
        "unix_now_s(",
        2,
        "boot_engine's first set_now, which the loop's first iteration overwrites from the \
         provider; and boot_unix, the uptime that @stats displays",
    ),
    (
        "crates/mush-server/src/world_task/commit_pipeline.rs",
        "unix_now_s(",
        2,
        "the store health's last-write stamp, which @stats and /healthz display",
    ),
    (
        "crates/mush-server/src/world_task/staff_ops.rs",
        "unix_now_ms(",
        1,
        "@ps renders its ages against the OS clock",
    ),
    (
        "crates/mush-server/src/world_task/staff_ops.rs",
        "unix_now_s(",
        1,
        "@stats renders the uptime against the OS clock",
    ),
    (
        "crates/mush-server/src/world_task/staff_ops.rs",
        "SystemTime::now(",
        1,
        "the economy journal's per-hour rate, a measurement",
    ),
    (
        "crates/mush-server/src/world_task/staff_ops.rs",
        "unix_now_s(",
        1,
        "an @sshkey record's `added` stamp, an account record read by the gateway",
    ),
    (
        "crates/mush-server/src/world_task/staff_ops.rs",
        "unix_now_s(",
        1,
        "a Discord link code's expiry, which the bridge compares on the OS clock",
    ),
    (
        "crates/mush-server/src/world_task/staff_ops.rs",
        "unix_now_s(",
        1,
        "a Discord ban record's `at` stamp, an account-side record",
    ),
];

/// Whether `rel` is a world-task or arena-task source file under a crate's `src/`.
fn in_scope(rel: &str) -> bool {
    let Some((_, under_src)) = rel.split_once("/src/") else {
        return false;
    };
    let parts: Vec<&str> = under_src.split('/').collect();
    let is_test_path = parts
        .iter()
        .any(|part| *part == "tests" || *part == "tests.rs" || part.ends_with("_tests.rs"));
    !is_test_path
        && parts
            .iter()
            .any(|part| part.starts_with("world_task") || part.starts_with("arena_task"))
}

/// Every needle call in scope, as `(file, needle)` to count, with each site's lines.
fn reads() -> (BTreeMap<(String, &'static str), Vec<usize>>, usize) {
    let root = scan::repo_root();
    let mut found: BTreeMap<(String, &'static str), Vec<usize>> = BTreeMap::new();
    let mut files_seen = 0;
    for krate in scan::crate_dirs(&root) {
        for file in scan::rust_files(&krate.join("src")) {
            let rel = scan::relative(&root, &file);
            if !in_scope(&rel) {
                continue;
            }
            let source = std::fs::read_to_string(&file).expect("a readable source file");
            let stripped = scan::strip_comments_and_literals(&source);
            if scan::file_is_test_only(&stripped) {
                continue;
            }
            files_seen += 1;
            let stripped = scan::blank_cfg_test_items(&stripped);
            for (number, line) in stripped.lines().enumerate() {
                for needle in NEEDLES {
                    let hits = line.matches(needle).count();
                    for _ in 0..hits {
                        found
                            .entry((rel.clone(), needle))
                            .or_default()
                            .push(number + 1);
                    }
                }
            }
        }
    }
    (found, files_seen)
}

#[test]
fn the_world_and_arena_tasks_read_the_os_clock_only_where_listed() {
    let (found, files_seen) = reads();
    assert!(
        files_seen >= 9,
        "the scan saw {files_seen} world_task/arena_task files; the paths moved"
    );
    let mut allowed: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for (file, needle, count, _) in ALLOWED {
        *allowed.entry((file.to_owned(), needle)).or_default() += count;
    }
    let mut wrong = Vec::new();
    for (key, lines) in &found {
        let listed = allowed.get(key).copied().unwrap_or(0);
        if lines.len() != listed {
            wrong.push(format!(
                "{}: {} calls of `{}` at lines {lines:?}; the list allows {listed}",
                key.0,
                lines.len(),
                key.1
            ));
        }
    }
    for (key, listed) in &allowed {
        if !found.contains_key(key) {
            wrong.push(format!(
                "{}: the list allows {listed} calls of `{}` and there are none; drop its row",
                key.0, key.1
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "a read of the OS clock in the world or arena task bypasses ServerDeps::time, which a \
         paused test rig sets to tokio's clock (doc 43c §2). Read the time through the \
         provider, or, for a read that only measures or displays, add a row to ALLOWED with \
         its reason:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_scope_is_what_the_gate_says() {
    assert!(in_scope("crates/mush-server/src/world_task.rs"));
    assert!(in_scope("crates/mush-server/src/world_task/staff_ops.rs"));
    assert!(in_scope("crates/tacsim/src/arena_task.rs"));
    assert!(in_scope("crates/tacsim/src/arena_task/supervisor.rs"));
    assert!(!in_scope("crates/mush-server/src/world_task/tests.rs"));
    assert!(!in_scope(
        "crates/mush-server/src/world_task/submission_disposition/tests/retry.rs"
    ));
    assert!(!in_scope(
        "crates/mush-server/tests/world_task/arena_bridge.rs"
    ));
    assert!(!in_scope("crates/mush-config/src/time_source.rs"));
    assert!(!in_scope("crates/mush-server/src/lib.rs"));
}
