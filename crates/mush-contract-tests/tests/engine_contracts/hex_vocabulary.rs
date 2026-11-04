//! **S7 — the H lens: no hex left (doc 14 D-14-4).**
//!
//! Square grids everywhere: a hex cannot be shown honestly in a text terminal,
//! so the owner struck hexes from every map, driver and document. This test
//! walks the tree for the *grid* vocabulary — the crate, the maps, the lock,
//! the pose, the driver name — and allows it only where it is history: the
//! spatial plan, the review that proposed the change, doc 14 and its owner
//! questions, and the design README's record of the decision. Hexadecimal
//! digests, colours and ids are not hexes and are not matched.

use mush_contract_tests::scan;

/// The files that may still say the word, because they record its removal.
const HISTORY: &[&str] = &[
    "docs/design/14-spatial-implementation-plan.md",
    "docs/design/11a-neighborhood-geography-review.md",
    "docs/design/14-spatial-room-system.md",
    "docs/design/14a-spatial-owner-questions.md",
    "docs/design/README.md",
    "crates/mush-contract-tests/tests/engine_contracts/hex_vocabulary.rs",
];

/// The grid vocabulary, lower-cased. Each is a substring of a lower-cased line;
/// the words are spelled with a split so this file's own text does not match
/// itself by accident.
fn vocabulary() -> Vec<String> {
    let h = String::from("he") + "x";
    vec![
        format!("{h}x"),
        format!("{h}es"),
        format!("{h} map"),
        format!("{h}-map"),
        format!("{h}map"),
        format!("{h} grid"),
        format!("{h}lock"),
        format!("{h}-lock"),
        format!("{h} lock"),
        format!("{h} pose"),
        format!("{h} math"),
        format!("{h} state"),
        format!("{h} render"),
        format!("{h} slice"),
        format!("{h} data"),
        format!("{h} coordinate"),
        format!("{h} movement"),
        format!("{h} blob"),
        format!("{h}-shaped"),
        format!("{h}/unit"),
        format!("n-{h} "),
        format!("{h}-by-{h}"),
        format!("per {h}"),
        format!("{h} per "),
        format!("tactics.{h}"),
        String::from("toy_") + "map",
        format!(", {h},"),
        format!("or {h}es"),
    ]
}

#[test]
fn no_hex_grid_vocabulary_outside_the_history() {
    let root = scan::repo_root();
    let mut files = Vec::new();
    for top in ["crates", "docs"] {
        files.extend(scan::files(
            &root.join(top),
            &["rs", "md", "toml", "json", "txt"],
            &["target", ".git", "node_modules", "dist"],
        ));
    }
    assert!(files.len() > 100, "the walk found the tree");
    let words = vocabulary();
    let mut offenders: Vec<String> = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("under the root")
            .to_string_lossy()
            .replace('\\', "/");
        if HISTORY.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            if let Some(word) = words.iter().find(|w| lower.contains(w.as_str())) {
                offenders.push(format!("{rel}:{}: {word:?} in {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "hex-grid vocabulary outside the history (D-14-4):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_history_files_exist() {
    let root = scan::repo_root();
    for file in HISTORY {
        assert!(
            root.join(file).is_file(),
            "{file} is where the history lives"
        );
    }
}
