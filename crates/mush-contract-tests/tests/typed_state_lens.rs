//! **The typed-state lens pin** (WS03-S7, completed by WS03-S13), in the
//! legacy-handler-allowlist style: a checked-in table beside this crate's
//! `Cargo.toml`, a scan that fails on an undeclared hit, and a message that
//! names the file to edit.
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
//! ## The three scans
//!
//! 1. **Detachment is declared.** Every `TypedStateLens::Detached(`
//!    construction under `crates/*/src` is listed in `typed-state-lens.toml`
//!    with a count and a reason. A new one fails until it is written down.
//! 2. **No name takes the old shape.** No function under `crates/*/src` —
//!    `pub`, `pub(crate)` or private — has an `Option<&…ActionStateStore>`
//!    parameter. WS03-S7 could only pin the public surface, because the
//!    crate-internal threading was still half converted; WS03-S13 finished it,
//!    so the shape is gone at every visibility and this scan says so.
//! 3. **No twin name is left.** The literals `_with_action_state` and
//!    `_via_action_state` appear nowhere under `crates/` but in this file. That
//!    is the naming half of the same finding: a PAIR of names is how the
//!    store-less default got in, and a suffix is how a pair announces itself.
//!
//! Scans 1 and 2 stop at `crates/*/src`: `crates/*/tests` are separate crates,
//! the same boundary the panic budget (WS01-S11) and the restriction lints
//! (WS01-S12) draw. Scan 3 covers **everything** under `crates/`, tests
//! included — a twin reintroduced in a test crate is still a twin.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The checked-in pin, beside this crate's `Cargo.toml`.
const PIN: &str = "typed-state-lens.toml";

/// The construction the first scan counts.
const DETACHED_NEEDLE: &str = "TypedStateLens::Detached(";

/// The qualifier tokens that may stand between the start of a line and the
/// `fn` that opens a function item, in any order Rust accepts.
const FN_QUALIFIERS: [&str; 6] = ["pub", "async", "const", "unsafe", "extern", "default"];

/// The twin suffixes scan 3 forbids.
const TWIN_SUFFIXES: [&str; 2] = ["_with_action_state", "_via_action_state"];

/// This file's own repo-relative path — the single exemption from scan 3,
/// which has to name the suffixes in order to forbid them. Spelled in full
/// rather than taken from `file!()`'s basename so that a future
/// `crates/<other>/src/typed_state_lens.rs` is scanned like anything else.
const SELF_PATH: &str = "crates/mush-contract-tests/tests/typed_state_lens.rs";

/// The offset of the `fn` keyword that opens `trimmed`, if any: `fn` as a
/// whole word, followed by the function's name.
fn find_fn_keyword(trimmed: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(offset) = trimmed[from..].find("fn ") {
        let at = from + offset;
        let boundary = at == 0
            || !trimmed[..at]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
        if boundary {
            return Some(at);
        }
        from = at + 3;
    }
    None
}

/// The offset of the `fn` keyword when `trimmed` — a source line with its
/// leading whitespace removed — opens a function item at ANY visibility.
///
/// Everything before the `fn` keyword must be a qualifier: a visibility
/// (`pub`, `pub(crate)`, `pub(super)`, `pub(in path)`), `async`, `const`,
/// `unsafe`, `extern "C"`, or `default`. That rejects the lines that merely
/// mention `fn` — a `type Cb = fn(…)` alias, a comment, a `dyn Fn(…)` bound
/// (capital F) — while accepting every real definition.
///
/// The OFFSET is the load-bearing half of the answer, not a convenience: a
/// visibility qualifier carries its own parentheses, so a caller that hunts
/// for the parameter list from the start of the line finds `(crate)` and
/// reads `pub(crate) fn f(store: Option<&ActionStateStore>)` as taking no
/// parameters at all. The search has to begin at the `fn`.
fn opens_a_function(trimmed: &str) -> Option<usize> {
    let at = find_fn_keyword(trimmed)?;
    let mut rest = &trimmed[..at];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Some(at);
        }
        if let Some(after) = rest.strip_prefix('"') {
            // An `extern "C"` ABI string.
            let end = after.find('"')?;
            rest = &after[end + 1..];
            continue;
        }
        let word_end = rest
            .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .unwrap_or(rest.len());
        let (word, mut tail) = rest.split_at(word_end);
        if !FN_QUALIFIERS.contains(&word) {
            return None;
        }
        // `pub(crate)`, `pub(super)`, `pub(in crate::foo)`.
        if word == "pub" && tail.starts_with('(') {
            let end = tail.find(')')?;
            tail = &tail[end + 1..];
        }
        rest = tail;
    }
}

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
// Scan 2: no name, at any visibility, still takes the old shape
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

/// The parameter list of the function head whose `fn` keyword sits at
/// `fn_start`, or `None` when the head never closes.
///
/// It cannot simply be the first `(` on the line. A visibility qualifier puts
/// one BEFORE the keyword (`pub(crate) fn f(…)`) and a generic bound puts one
/// after the name but inside the angle brackets (`fn f<F: Fn(&X)>(…)`), so the
/// walk starts at the keyword and takes the first `(` at angle depth zero. The
/// `-` guard keeps a bound's return arrow (`Fn(u8) -> u8`) from closing a
/// generic list that is still open.
fn parameter_list(text: &str, fn_start: usize) -> Option<&str> {
    let head = text.get(fn_start..)?;
    let mut angle = 0usize;
    let mut previous = ' ';
    for (offset, ch) in head.char_indices() {
        match ch {
            '<' => angle += 1,
            '>' if previous != '-' => angle = angle.saturating_sub(1),
            '(' if angle == 0 => return paren_group(text, fn_start + offset),
            _ => {}
        }
        previous = ch;
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

/// Scan 2's body, over whatever sources it is handed: every function head, at
/// every visibility, whose parameter list declares an `Option<&…Store>`.
///
/// It is a free function rather than a loop inside the test so the test below
/// can run the WHOLE scan — head matcher, parameter-list walk and type
/// normaliser together — over a synthetic source. Exercising the matcher on
/// its own is exactly what let a `pub(crate)` hole through review once: the
/// matcher knew every visibility while the scan around it did not.
fn optional_store_offenders(sources: &[(String, String)]) -> Vec<String> {
    let mut offenders = Vec::new();
    for (relative, text) in sources {
        let mut offset = 0usize;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if let Some(fn_at) = opens_a_function(trimmed)
                && let Some(params) = parameter_list(text, offset + indent + fn_at)
                && takes_optional_store(params)
            {
                let name = trimmed.trim_end();
                offenders.push(format!("  {relative}: {name}"));
            }
            offset += line.len();
        }
    }
    offenders
}

#[test]
fn no_function_at_any_visibility_takes_an_optional_action_state_store() {
    let root = repo_root();
    let offenders = optional_store_offenders(&production_sources(&root));
    assert!(
        offenders.is_empty(),
        "a function still takes `Option<&ActionStateStore>`. That is the shape WS03-S7 \
         removed and WS03-S13 finished removing: a caller can pass `None` and get the \
         store-less perception, dispatch or veto answer without saying so. Take \
         `mush_core::TypedStateLens` instead — `Live(store)`, or `Detached(reason)` \
         naming why there is none.\n{}",
        offenders.join("\n")
    );
}

/// The function-head matcher itself, since scan 2's reach is only as good as
/// it is: WS03-S13 widened this scan from `pub fn` to every visibility, and a
/// matcher that quietly missed `pub(crate) fn` would make the widening a lie.
#[test]
fn the_function_head_matcher_recognises_every_visibility_it_must() {
    for head in [
        "fn plain(",
        "pub fn public(",
        "pub(crate) fn crate_private(",
        "pub(super) fn super_private(",
        "pub(in crate::action_state) fn scoped(",
        "async fn asynchronous(",
        "pub async fn public_async(",
        "const fn constant(",
        "pub const fn public_const(",
        "unsafe fn unsound(",
        "pub unsafe fn public_unsafe(",
        "pub extern \"C\" fn abi(",
    ] {
        assert!(
            opens_a_function(head).is_some(),
            "the matcher must recognise `{head}`"
        );
    }
    for head in [
        "/// fn documented(",
        "// fn commented(",
        "type Callback = fn(",
        "handler: Box<dyn Fn(",
        "let it = something.fn_like(",
        "struct Holder {",
        "",
    ] {
        assert!(
            opens_a_function(head).is_none(),
            "the matcher must NOT flag `{head}`"
        );
    }
}

/// The WHOLE of scan 2 over a synthetic source, one visibility at a time.
///
/// The test above proves the head matcher knows `pub(crate) fn`; this one
/// proves the scan BUILT on it does. They came apart once: the scan hunted the
/// parameter list from the start of the line, so a visibility's own
/// parentheses — `pub(crate)` — were read as the parameter list and every
/// crate-private offender walked straight through a green suite. Since
/// crate-private is precisely the population WS03-S13 widened the scan to
/// cover, the pin was blind to almost all of what it claimed.
#[test]
fn the_scan_flags_an_optional_store_behind_every_visibility_and_generic() {
    for head in [
        "fn plain",
        "pub fn public",
        "pub(crate) fn crate_private",
        "pub(super) fn super_private",
        "pub(in crate::action_state) fn scoped",
        "pub async fn public_async",
        "pub(crate) const fn crate_const",
        "pub(crate) unsafe extern \"C\" fn crate_abi",
        "pub(crate) fn generic<F: Fn(&Registry) -> bool>",
        "pub(crate) fn lifetimed<'a>",
    ] {
        let text = format!(
            "impl Thing {{\n    {head}(action_state: Option<&ActionStateStore>) {{}}\n}}\n"
        );
        let sources = vec![("crates/synthetic/src/lib.rs".to_string(), text)];
        let offenders = optional_store_offenders(&sources);
        assert_eq!(
            offenders.len(),
            1,
            "scan 2 must flag `{head}(action_state: Option<&ActionStateStore>)`, not just \
             the matcher underneath it; it found {offenders:?}"
        );
    }
    // And the shapes it must leave alone, through the same whole scan.
    for head in [
        "pub(crate) fn lens_taking(lens: TypedStateLens<'_>)",
        "pub(crate) fn borrowed(action_state: &ActionStateStore)",
        "type Callback = fn(action_state: Option<&ActionStateStore>);",
    ] {
        let sources = vec![(
            "crates/synthetic/src/lib.rs".to_string(),
            format!("{head} {{}}\n"),
        )];
        assert!(
            optional_store_offenders(&sources).is_empty(),
            "scan 2 must NOT flag `{head}`"
        );
    }
}

// ---------------------------------------------------------------------------
// Scan 3: no twin name is left anywhere under crates/
// ---------------------------------------------------------------------------

/// Every `.rs` file under `crates/`, production and test alike, keyed by its
/// repo-relative path with forward slashes. Scan 3's reach is deliberately
/// wider than scans 1 and 2's: a twin reintroduced in a test crate is a twin.
fn every_source(root: &Path) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    for path in rust_files(&root.join("crates")) {
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
    sources
}

/// WS03-S13. `mush_core` used to carry PAIRS of names around the same call —
/// `can_perceive` beside `can_perceive_with_action_state`, and thirteen more
/// families like it — where the short name silently meant "no typed state".
/// Collapsing them onto [`mush_core::TypedStateLens`] is only durable if the
/// suffix cannot come back, because the suffix is what a reintroduced pair
/// looks like on the way in.
#[test]
fn no_action_state_twin_names_remain() {
    let root = repo_root();
    assert!(
        root.join(SELF_PATH).is_file(),
        "{SELF_PATH} is this test's own repo-relative path and the ONE file the scan \
         exempts; if the file moved, move the constant with it — matching on the \
         bare basename would exempt any other `typed_state_lens.rs` in the \
         workspace too."
    );
    let mut offenders = Vec::new();
    for (relative, text) in every_source(&root) {
        if relative == SELF_PATH {
            continue;
        }
        for suffix in TWIN_SUFFIXES {
            let count = text.matches(suffix).count();
            if count > 0 {
                offenders.push(format!("  {relative}: {count} × `{suffix}`"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a `_with_action_state` / `_via_action_state` twin is back. Those suffixes named \
         the second half of a PAIR: one spelling that passed the store and a shorter one \
         that passed `None`, with nothing making a caller choose — which is how a \
         perception hook came to answer the wrong verdict rather than a degraded one. \
         Take one name and one `mush_core::TypedStateLens` parameter instead.\n{}",
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
