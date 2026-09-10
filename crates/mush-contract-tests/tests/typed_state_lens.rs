//! **The typed-state lens pin** (WS03-S7), in the legacy-handler-allowlist
//! style: a checked-in table beside this crate's `Cargo.toml`, a scan that
//! fails on an undeclared hit, and a message that names the file to edit.
//!
//! ## What the finding was
//!
//! Whether a call could reach engine-owned typed state used to be an
//! `Option<&ActionStateStore>` threaded through the engine behind PAIRS of
//! public names: `can_perceive` passing `None` beside
//! `can_perceive_with_action_state` passing the store, and the same shape
//! around `is_concealed`, `perceives_via`, `check_move_attempt`,
//! `check_carry_attempt`, `look_rendered_lines`, `output::room_view`,
//! `movement::traverse_exit` and `movement::go_home`.
//!
//! The store-less spelling was the shorter, more reachable name, which made
//! "no typed state" the accidental default. That is not a degraded answer: a
//! `Perceive` hook that reads a typed family (WoD's Obfuscate cache is the
//! live one) abstains when it cannot read it, so a production caller that
//! reached for the short name got the **wrong perception verdict** — a
//! concealed subject rendered, or a visible one hidden — with nothing in the
//! type system objecting.
//!
//! `mush_core::TypedStateLens` collapses each pair into one name taking one
//! lens: `Live(store)`, or `Detached(reason)` where the reason is one of the
//! four in `DetachedTypedState`. A store-less call now has to say so.
//!
//! ## The two scans
//!
//! 1. **Detachment is declared.** Every `TypedStateLens::Detached(`
//!    construction under `crates/*/src` is listed in `typed-state-lens.toml`
//!    with a count and a reason. A new one fails until it is written down.
//! 2. **No public name takes the old shape.** No `pub fn` under
//!    `crates/*/src` has an `Option<&…ActionStateStore>` parameter, so the
//!    twins WS03-S13 has not converted yet cannot be reached by a store-less
//!    public name in the meantime.
//!
//! `crates/*/tests` is deliberately out of scope for both: those are separate
//! crates, the same boundary the panic budget (WS01-S11) and the restriction
//! lints (WS01-S12) draw.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The checked-in pin, beside this crate's `Cargo.toml`.
const PIN: &str = "typed-state-lens.toml";

/// The construction the first scan counts.
const DETACHED_NEEDLE: &str = "TypedStateLens::Detached(";

/// The heads that open a public function. `pub(crate) fn` and `pub(super) fn`
/// deliberately do not match: this scan is about the names another crate can
/// reach, and the crate-internal threading is WS03-S13's to finish.
const PUB_FN_HEADS: [&str; 4] = [
    "pub fn ",
    "pub async fn ",
    "pub const fn ",
    "pub unsafe fn ",
];

#[derive(Debug, Deserialize)]
struct Pin {
    detached: Vec<DetachedRow>,
}

#[derive(Debug, Deserialize)]
struct DetachedRow {
    file: String,
    count: usize,
    reason: String,
}

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, sorted, so the walk is reproducible.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files
}

/// Every production source file in the workspace: `crates/*/src/**/*.rs`,
/// keyed by its repo-relative path with forward slashes.
fn production_sources(root: &Path) -> Vec<(String, String)> {
    let crates_dir = root.join("crates");
    let entries = fs::read_dir(&crates_dir)
        .unwrap_or_else(|err| panic!("{} is not readable: {err}", crates_dir.display()));
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    dirs.sort();
    let mut sources = Vec::new();
    for dir in dirs {
        if !dir.join("Cargo.toml").is_file() {
            continue;
        }
        for path in rust_files(&dir.join("src")) {
            let text =
                fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
                .replace('\\', "/");
            sources.push((relative, text));
        }
    }
    sources
}

/// Read and parse the pin.
fn read_pin(root: &Path) -> Pin {
    let path = root.join("crates").join("mush-contract-tests").join(PIN);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    toml::from_str(&text).unwrap_or_else(|err| panic!("{} does not parse: {err}", path.display()))
}

// ---------------------------------------------------------------------------
// Scan 1: every detached lens is declared
// ---------------------------------------------------------------------------

#[test]
fn every_detached_typed_state_lens_is_declared_in_the_pin() {
    let root = repo_root();
    let pin = read_pin(&root);

    let mut declared: BTreeMap<&str, &DetachedRow> = BTreeMap::new();
    for row in &pin.detached {
        assert!(
            declared.insert(row.file.as_str(), row).is_none(),
            "{PIN} lists {} twice; one row per file, with the total count",
            row.file
        );
        assert!(
            !row.reason.trim().is_empty(),
            "{PIN}: {} needs a reason, not an empty string",
            row.file
        );
        assert!(
            row.count > 0,
            "{PIN}: {} is pinned at zero — delete the row instead",
            row.file
        );
    }

    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for (relative, text) in production_sources(&root) {
        let count = text.matches(DETACHED_NEEDLE).count();
        if count > 0 {
            found.insert(relative, count);
        }
    }

    let mut complaints = Vec::new();
    for (file, count) in &found {
        match declared.get(file.as_str()) {
            None => complaints.push(format!(
                "  {file}: {count} `{DETACHED_NEEDLE}` — undeclared. A call that cannot \
                 reach typed state must say WHY; add a row to {PIN} naming the reason, or \
                 thread the caller's live lens instead."
            )),
            Some(row) if row.count != *count => complaints.push(format!(
                "  {file}: {count} `{DETACHED_NEEDLE}`, {PIN} pins {}. Raising a count is a \
                 reviewed diff: say in the row why that file needs another detached lens.",
                row.count
            )),
            Some(_) => {}
        }
    }
    for file in declared.keys() {
        if !found.contains_key(*file) {
            complaints.push(format!(
                "  {file}: {PIN} pins a count but the file has none left. Delete the row — \
                 that is what finishing the collapse looks like."
            ));
        }
    }

    assert!(
        complaints.is_empty(),
        "the typed-state lens pin ({PIN}) no longer matches the tree:\n{}",
        complaints.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Scan 2: no public name still takes the old shape
// ---------------------------------------------------------------------------

/// The parenthesised group opening at or after `start`, or `None` when the
/// parentheses do not close.
fn paren_group(text: &str, start: usize) -> Option<&str> {
    let open = start + text.get(start..)?.find('(')?;
    let mut depth = 0usize;
    for (offset, ch) in text.get(open..)?.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(open + 1..open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether a parameter list declares an `Option<&ActionStateStore>` in any of
/// its spellings — bare, lifetime-bound, module-qualified, or wrapped across
/// lines by rustfmt.
///
/// Lifetimes are dropped and whitespace removed first, so every spelling
/// normalises to `Option<&some::path::ActionStateStore>`. Dropping lifetimes
/// BEFORE squeezing matters: squeezing alone glues `&'a ActionStateStore` into
/// `&'aActionStateStore`, which no suffix match can pick apart.
fn takes_optional_store(params: &str) -> bool {
    let mut normalized = String::with_capacity(params.len());
    let mut chars = params.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            while chars
                .peek()
                .is_some_and(|next| next.is_alphanumeric() || *next == '_')
            {
                chars.next();
            }
            continue;
        }
        if !ch.is_whitespace() {
            normalized.push(ch);
        }
    }

    let mut rest = normalized.as_str();
    while let Some(at) = rest.find("Option<&") {
        let mut path = &rest[at + "Option<&".len()..];
        // Any number of `foo::` path segments, then the type itself.
        while let Some(colons) = path.find("::") {
            let segment = &path[..colons];
            if segment.is_empty() || !segment.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                break;
            }
            path = &path[colons + 2..];
        }
        if let Some(after) = path.strip_prefix("ActionStateStore")
            && (after.starts_with('>') || after.starts_with(",>"))
        {
            return true;
        }
        rest = &rest[at + "Option<&".len()..];
    }
    false
}

#[test]
fn no_public_function_still_takes_an_optional_action_state_store() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for (relative, text) in production_sources(&root) {
        let mut offset = 0usize;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if PUB_FN_HEADS.iter().any(|head| trimmed.starts_with(head))
                && let Some(params) = paren_group(&text, offset + indent)
                && takes_optional_store(params)
            {
                let name = trimmed.trim_end();
                offenders.push(format!("  {relative}: {name}"));
            }
            offset += line.len();
        }
    }
    assert!(
        offenders.is_empty(),
        "a public function still takes `Option<&ActionStateStore>`. That is the shape \
         WS03-S7 removed: a caller can pass `None` and get the store-less perception or \
         veto answer without saying so. Take `mush_core::TypedStateLens` instead — \
         `Live(store)`, or `Detached(reason)` naming why there is none.\n{}",
        offenders.join("\n")
    );
}

/// The parameter matcher itself, since scan 2 is only as good as it is.
#[test]
fn the_optional_store_matcher_recognises_every_spelling_it_must() {
    for shape in [
        "reg: &Registry, action_state: Option<&ActionStateStore>",
        "action_state: Option<&'a ActionStateStore>",
        "action_state: Option<&crate::action_state::ActionStateStore>",
        "action_state: Option<&'a crate::action_state::ActionStateStore>",
        "action_state: Option<&mush_core::ActionStateStore>",
        "\n    action_state: Option<\n        &crate::action_state::ActionStateStore,\n    >",
    ] {
        assert!(
            takes_optional_store(shape),
            "the matcher must recognise `{shape}`"
        );
    }
    for shape in [
        "lens: TypedStateLens<'_>",
        "action_state: &ActionStateStore",
        "state: &mut ActionStateStore",
        "seen: Option<&ActionStateBootRecord>",
        "",
    ] {
        assert!(
            !takes_optional_store(shape),
            "the matcher must NOT flag `{shape}`"
        );
    }
}
