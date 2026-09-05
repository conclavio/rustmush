//! Architectural tripwire for interface cut I1 (docs/interface/02 §3.1): a
//! command-side sheet write goes through `SheetStore::put_in`, which emits
//! the `AttrSet` event the `<ruleset>.vitals` and `<ruleset>.sheet`
//! republishers subscribe to. A `put_by(ctx.w, …)` in ruleset command code
//! is a write nobody hears about — the client keeps a stale sheet and stale
//! vitals until its next poll — so this probe refuses it.
//!
//! `put_by` itself stays legitimate where there is no `Ctx` to emit from
//! (NPC sheet bootstrap, the store's own helpers).

use std::path::{Path, PathBuf};

const FORBIDDEN: &str = ".put_by(ctx.w";

fn rust_sources(root: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot scan {}: {error}", root.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("cannot read entry in {}: {error}", root.display()));
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let kind = entry
            .file_type()
            .unwrap_or_else(|error| panic!("cannot inspect {}: {error}", path.display()));
        if kind.is_dir() {
            if entry.file_name() == "tests" {
                continue;
            }
            rust_sources(&path, files);
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn ruleset_command_code_writes_sheets_through_put_in() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for crate_dir in ["ruleset-wod", "ruleset-shadowrun"] {
        rust_sources(&crates.join(crate_dir).join("src"), &mut files);
    }
    files.push(crates.join("mush-ruleset").join("src").join("commands.rs"));

    let mut offenders = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        for (index, line) in source.lines().enumerate() {
            if line.contains(FORBIDDEN) {
                offenders.push(format!("{}:{}: {}", file.display(), index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "sheet writes in command code must use SheetStore::put_in (I1):\n{}",
        offenders.join("\n")
    );
}
