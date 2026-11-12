//! **Nothing outside a space mind writes its memory, or reads it to decide** (doc 44 unit
//! MW.9; Q-44-24).
//!
//! Until MW.9 the customs picket's host wrote `stage` into the corvette mind's memory when a
//! carrier was cleared, and read `stage` back out of it to decide that a carrier had hove to
//! or had run. A mind's memory is the mind's own: what the host knows reaches a mind through
//! its view (`crates/tacsim/src/space/view.rs`), and what the host decides it decides on the
//! bubble's state.
//!
//! Two rules over `crates/tacsim/src/space/`, tests aside:
//!
//! 1. Nothing there builds or opens a memory. `encode_memory` is the one way to make the
//!    bytes and `decode_memory` the one way to read a key, so neither is named, and neither
//!    is `mush_mind::memory`.
//! 2. A mind's memory field is assigned only what its own think returned: the right-hand side
//!    of every assignment to one is the think result's `memory`.

use mush_contract_tests::scan;

/// The fields that hold a space mind's memory between thinks.
const MEMORY_FIELDS: [&str; 4] = [
    "corvette_memory",
    "raider_memory",
    "interdictor_memory",
    "mind_memory",
];

/// The right-hand sides a memory field may be assigned.
const THINK_RESULTS: [&str; 2] = ["res.memory", "result.memory"];

/// Every production line of `crates/tacsim/src/space/`, as `(file, line number, text)`, with
/// comments, literals and `#[cfg(test)]` items blanked.
fn space_lines() -> Vec<(String, usize, String)> {
    let root = scan::repo_root();
    let space = root.join("crates/tacsim/src/space");
    let files = scan::rust_files(&space);
    assert!(
        files.len() >= 10,
        "crates/tacsim/src/space moved: {} files",
        files.len()
    );
    let mut lines = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("a readable source file");
        let stripped = scan::blank_cfg_test_items(&scan::strip_comments_and_literals(&source));
        let name = scan::relative(&root, &file);
        for (number, line) in stripped.lines().enumerate() {
            lines.push((name.clone(), number + 1, line.to_owned()));
        }
    }
    lines
}

#[test]
fn no_space_code_builds_or_opens_a_minds_memory() {
    let mut found = Vec::new();
    for (file, number, line) in space_lines() {
        for banned in ["encode_memory", "decode_memory", "mush_mind::memory"] {
            if line.contains(banned) {
                found.push(format!("{file}:{number}: {banned}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "a space mind's memory is its own; tell the mind through its view instead:\n{}",
        found.join("\n")
    );
}

/// The right-hand side of an assignment to `field` on `line`, if the line makes one: the
/// text after a single `=` that follows the field's name, up to the `;`.
fn assigned(line: &str, field: &str) -> Option<String> {
    let at = line.find(field)?;
    let before = line[..at].chars().next_back();
    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let rest = line[at + field.len()..].trim_start();
    let value = rest.strip_prefix('=')?;
    if value.starts_with('=') {
        return None;
    }
    Some(value.trim().trim_end_matches(';').trim().to_owned())
}

#[test]
fn a_space_minds_memory_is_assigned_only_what_its_own_think_returned() {
    let mut assignments = 0;
    let mut wrong = Vec::new();
    for (file, number, line) in space_lines() {
        for field in MEMORY_FIELDS {
            let Some(value) = assigned(&line, field) else {
                continue;
            };
            assignments += 1;
            if !THINK_RESULTS.contains(&value.as_str()) {
                wrong.push(format!("{file}:{number}: {field} = {value}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a space mind's memory may be assigned only its own think's `memory`:\n{}",
        wrong.join("\n")
    );
    // The corvette, the raider, the interdictor, the backup mind and the drone.
    assert!(
        assignments >= 5,
        "the scan found {assignments} assignments to a mind's memory; it has stopped seeing them"
    );
}

#[test]
fn the_scan_reads_an_assignment_and_not_a_comparison() {
    assert_eq!(
        assigned(
            "    picket.corvette_memory = res.memory;",
            "corvette_memory"
        )
        .as_deref(),
        Some("res.memory")
    );
    assert_eq!(
        assigned(
            "    picket.corvette_memory = encode(&mem);",
            "corvette_memory"
        )
        .as_deref(),
        Some("encode(&mem)")
    );
    assert_eq!(
        assigned("    if state.mind_memory != result.memory {", "mind_memory"),
        None
    );
    assert_eq!(
        assigned("    if a.mind_memory == b.mind_memory {", "mind_memory"),
        None
    );
    assert_eq!(
        assigned("        &picket.corvette_memory,", "corvette_memory"),
        None
    );
    assert_eq!(assigned("    other_mind_memory = x;", "mind_memory"), None);
}
