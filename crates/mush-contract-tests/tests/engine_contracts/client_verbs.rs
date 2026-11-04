//! **The client-to-verb seam** (doc 43 cut TS.7, §5.2).
//!
//! The 35b `DOCK` dead end had three parts: both clients sent a string no verb
//! parsed, `DockingClientSession` had no production caller, and every space
//! and pixel test stayed green. Nothing tied a client's strings to the
//! registry the server composes. This file is that tie, in two contracts:
//!
//! 1. **Every command a client emits is a registered verb.** The TUI
//!    (`crates/mush-net-ssh/src/tui/`) and the web client (`web/src/`) are
//!    scanned for string literals shaped like a command line — a `+verb` or
//!    `@verb` (with its `/switches`), a cockpit-style upper-case head
//!    (`FIRE HOLD`, `HEADING {} QUIET`), or the argument of a key table's
//!    `Run`/`Prefill` or a `cmd:`/`line:` entry — and each head must resolve in
//!    [`production_composition`] the way dispatch resolves it (`lookup`: exact
//!    name or alias, else an unambiguous prefix for a bare verb), with every
//!    literal switch one the verb declares. A head that is not a command (a
//!    label that happens to be upper case) or a known dead end goes in
//!    `client-verbs-allowlist.toml` with its reason; an unused row fails, so
//!    the list cannot outlive what it excuses.
//! 2. **Every session type a client crate defines has a production
//!    constructor.** A `pub struct *Session*` in the net crates or the pixel
//!    crates whose name is never constructed (`Name::…`, `Name {`) in any
//!    workspace crate's non-test source is a type only tests build.
//!
//! A scan, not a parser: a command assembled at run time from pieces (the
//! detachment's server-sent `wilco` line) is invisible to it, and a Rust file's
//! tests are cut at its first `#[cfg(test)]` line.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use mush_contract_tests::scan;

use crate::production_registry::production_composition;

/// Where the clients emit command lines.
const CLIENT_DIRS: [&str; 2] = ["crates/mush-net-ssh/src/tui", "web/src"];

/// The crates whose `pub struct *Session*` types are a client's sessions.
const SESSION_CRATES: [&str; 6] = [
    "crates/mush-net-ssh",
    "crates/mush-net-web",
    "crates/mush-net-discord",
    "crates/mush-gateway",
    "crates/mush-pixel",
    "crates/mush-pixel-wasm",
];

const ALLOWLIST: &str = "client-verbs-allowlist.toml";

#[derive(Debug, serde::Deserialize)]
struct Allowlist {
    #[serde(rename = "allow", default)]
    rows: Vec<AllowRow>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowRow {
    head: String,
    /// Empty: the head itself is excused (it is no verb). Set: only that
    /// switch of a verb that is registered.
    #[serde(default)]
    switch: String,
    reason: String,
}

/// One command-shaped literal a client emits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Emitted {
    /// `+helm`, `@mail`, `FIRE`, `look`.
    head: String,
    /// The literal switches after the head (`+helm/dock` → `dock`); a
    /// templated one (`+tv/{n}`) ends the list.
    switches: Vec<String>,
    /// `<file>:<line>`.
    at: String,
}

/// The string literals of a source, with their 1-based lines: `"…"`, `'…'`
/// and `` `…` ``, escapes skipped. A literal that opens on a comment line
/// (`//`, `*`, `/*`, `<!--`) is not a client's.
fn literals(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if ["//", "*", "/*", "<!--"]
            .iter()
            .any(|marker| trimmed.starts_with(marker))
        {
            continue;
        }
        let mut chars = line.char_indices().peekable();
        while let Some((at, ch)) = chars.next() {
            if ch == '/' && line[at..].starts_with("//") {
                break;
            }
            if !matches!(ch, '"' | '\'' | '`') {
                continue;
            }
            // A Rust lifetime or char literal is not a command line.
            if ch == '\'' && line[..at].ends_with(|c: char| c.is_alphanumeric() || c == '&') {
                continue;
            }
            let mut text = String::new();
            let mut closed = false;
            while let Some((_, next)) = chars.next() {
                if next == '\\' {
                    chars.next();
                    continue;
                }
                if next == ch {
                    closed = true;
                    break;
                }
                text.push(next);
            }
            if closed {
                out.push((index + 1, text));
            }
        }
    }
    out
}

/// What a table entry's argument looks like in each client: the key tables'
/// `Run("…")`/`Prefill("…")`/`PrefillRow("…")` (TUI) and `cmd: "…"` /
/// `line: "…"` (web), and the send helpers.
const TABLE_CALLS: [&str; 9] = [
    "Run(",
    "Prefill(",
    "PrefillRow(",
    "cmd: ",
    "line: ",
    "sendLine(",
    "submit(",
    "helm(",
    "orders.push(",
];

/// The command a literal is, if it is shaped like one. `in_table` is true
/// when the literal is a key table's or a send helper's argument, where a
/// lower-case bare verb counts too.
fn command_of(literal: &str, in_table: bool) -> Option<(String, Vec<String>)> {
    let first = literal.split_whitespace().next()?;
    let templated = |part: &str| part.contains('{') || part.contains('$');
    if let Some(rest) = first.strip_prefix(['+', '@']) {
        if !rest.starts_with(|c: char| c.is_ascii_lowercase()) {
            return None;
        }
        let mut parts = first.split('/');
        let head = parts.next()?.to_string();
        if templated(&head) {
            return None;
        }
        let switches = parts
            .take_while(|part| !part.is_empty() && !templated(part))
            .map(str::to_lowercase)
            .collect();
        return Some((head.to_lowercase(), switches));
    }
    let caps = first.len() >= 3 && first.chars().all(|c| c.is_ascii_uppercase());
    // A cockpit line is upper case: `FIRE HOLD`, `HEADING {} QUIET`. One
    // upper-case word alone outside a table or a send is a label ("ARMOUR",
    // "HEAT"), not a command; with a second word, or as a table's or a send's
    // argument, it is a line a client sends.
    let words = literal.split_whitespace().count();
    if caps && literal.starts_with(first) && (in_table || words >= 2) {
        return Some((first.to_string(), Vec::new()));
    }
    if in_table
        && literal.starts_with(first)
        && first.len() >= 2
        && first.chars().all(|c| c.is_ascii_lowercase())
    {
        return Some((first.to_string(), Vec::new()));
    }
    None
}

fn client_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = scan::files(&root.join(CLIENT_DIRS[0]), &["rs"], &["target"]);
    files.extend(
        scan::files(
            &root.join(CLIENT_DIRS[1]),
            &["ts", "svelte"],
            &["node_modules"],
        )
        .into_iter()
        .filter(|path| {
            let name = path.to_string_lossy();
            !name.ends_with(".test.ts") && !name.contains("fixtures")
        }),
    );
    files
}

/// Every command-shaped literal the two clients emit.
fn emitted(root: &Path) -> Vec<Emitted> {
    let mut out = Vec::new();
    for file in client_files(root) {
        let Ok(mut source) = fs::read_to_string(&file) else {
            continue;
        };
        let relative = scan::relative(root, &file);
        if relative.ends_with(".rs") {
            if relative.ends_with("/tests.rs") || relative.contains("/tests/") {
                continue;
            }
            if let Some(cut) = source.find("\n#[cfg(test)]") {
                source.truncate(cut);
            }
        }
        let lines: Vec<&str> = source.lines().collect();
        for (line, literal) in literals(&source) {
            let text = lines.get(line - 1).copied().unwrap_or_default();
            // A module path (`import … from "@codemirror/state"`) is no command.
            if text.trim_start().starts_with("import ") || text.contains(" from ") {
                continue;
            }
            let quoted = [
                format!("\"{literal}"),
                format!("'{literal}"),
                format!("`{literal}"),
            ];
            let in_table = TABLE_CALLS
                .iter()
                .any(|call| quoted.iter().any(|q| text.contains(&format!("{call}{q}"))));
            if let Some((head, switches)) = command_of(&literal, in_table) {
                out.push(Emitted {
                    head,
                    switches,
                    at: format!("{relative}:{line}"),
                });
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `(head, switch)` → reason, lower-cased; an empty switch excuses the head.
fn allowlist(root: &Path) -> BTreeMap<(String, String), String> {
    let path = root.join("crates/mush-contract-tests").join(ALLOWLIST);
    let text = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", path.display()));
    let parsed: Allowlist =
        toml::from_str(&text).unwrap_or_else(|error| panic!("{ALLOWLIST}: {error}"));
    let mut rows = BTreeMap::new();
    for row in parsed.rows {
        assert!(
            !row.reason.trim().is_empty(),
            "{ALLOWLIST}: `{}` needs a reason",
            row.head
        );
        assert!(
            rows.insert(
                (row.head.to_lowercase(), row.switch.to_lowercase()),
                row.reason
            )
            .is_none(),
            "{ALLOWLIST}: `{}` `{}` is listed twice",
            row.head,
            row.switch
        );
    }
    rows
}

/// **TS.7, the first contract.** Every command head a client emits resolves
/// in the production registry the way dispatch resolves it, with its literal
/// switches declared; the exceptions are named, with reasons, and used.
#[test]
fn every_command_a_client_emits_is_a_registered_verb() {
    let root = scan::repo_root();
    let composition = production_composition();
    let registry = composition.registry();
    let allowed = allowlist(&root);
    let commands = emitted(&root);
    let heads: BTreeSet<&str> = commands.iter().map(|c| c.head.as_str()).collect();
    for floor in ["+tac", "fire", "look", "DOCK"] {
        assert!(
            heads.iter().any(|head| head.eq_ignore_ascii_case(floor)),
            "the scan found no `{floor}` in the clients ({} heads): it is broken, not the clients",
            heads.len()
        );
    }

    let mut unknown: Vec<String> = Vec::new();
    let mut used: BTreeSet<(String, String)> = BTreeSet::new();
    for command in &commands {
        let head = command.head.to_lowercase();
        let key = (head.clone(), String::new());
        if allowed.contains_key(&key) {
            used.insert(key);
            continue;
        }
        let spec = match registry.lookup(&command.head) {
            Ok((spec, _)) => spec,
            Err(error) => {
                unknown.push(format!(
                    "{}: `{}` is no registered verb ({error:?})",
                    command.at, command.head
                ));
                continue;
            }
        };
        for switch in &command.switches {
            if !spec.switches.iter().any(|s| s.eq_ignore_ascii_case(switch)) {
                let key = (head.clone(), switch.clone());
                if allowed.contains_key(&key) {
                    used.insert(key);
                    continue;
                }
                unknown.push(format!(
                    "{}: `{}` has no switch /{switch}",
                    command.at, command.head
                ));
            }
        }
    }
    let unused: Vec<&(String, String)> =
        allowed.keys().filter(|key| !used.contains(*key)).collect();
    assert!(
        unknown.is_empty() && unused.is_empty(),
        "the clients and the registry disagree (doc 43 TS.7):\n  {}\n\
         allowlist rows no client line uses (remove them): {unused:?}\n\
         A command a client sends must be a verb the server registers; a literal that \
         is not a command, or a known dead end, goes in {ALLOWLIST} with its reason.",
        unknown.join("\n  ")
    );
}

/// **TS.7, the second contract.** A `pub struct *Session*` of a client crate
/// is constructed somewhere in production source: `Name::` or `Name {` in a
/// workspace crate's `src/`, with tests, comments and literals blanked, other
/// than on its own `struct`/`impl` lines.
#[test]
fn every_client_session_type_has_a_production_constructor() {
    let root = scan::repo_root();
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    for krate in SESSION_CRATES {
        for file in scan::files(&root.join(krate).join("src"), &["rs"], &["target"]) {
            let source = fs::read_to_string(&file).unwrap_or_default();
            let code = scan::blank_cfg_test_items(&scan::strip_comments_and_literals(&source));
            if scan::file_is_test_only(&code) {
                continue;
            }
            for line in code.lines() {
                let Some(rest) = line.trim_start().strip_prefix("pub struct ") else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if name.contains("Session") {
                    types.insert(name, scan::relative(&root, &file));
                }
            }
        }
    }
    assert!(
        types.contains_key("DockingClientSession") && types.contains_key("TuiSession"),
        "the session scan found {types:?}: it is broken, not the crates"
    );

    let mut production = String::new();
    for krate in scan::crate_dirs(&root) {
        for file in scan::files(&krate.join("src"), &["rs"], &["target"]) {
            let source = fs::read_to_string(&file).unwrap_or_default();
            let code = scan::blank_cfg_test_items(&scan::strip_comments_and_literals(&source));
            if !scan::file_is_test_only(&code) {
                production.push_str(&code);
                production.push('\n');
            }
        }
    }
    let mut unbuilt = Vec::new();
    for (name, file) in &types {
        let constructed = production.lines().any(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("pub struct ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("impl")
            {
                return false;
            }
            line.match_indices(name.as_str()).any(|(at, _)| {
                let before = line[..at].chars().next_back();
                let after = &line[at + name.len()..];
                !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    && (after.starts_with("::") || after.trim_start().starts_with('{'))
            })
        });
        if !constructed {
            unbuilt.push(format!("{name} ({file})"));
        }
    }
    assert!(
        unbuilt.is_empty(),
        "a client session type no production code constructs (doc 43 TS.7, the 35b \
         `DockingClientSession` class): wire it, or delete it:\n  {}",
        unbuilt.join("\n  ")
    );
}

#[test]
fn the_command_scan_reads_heads_switches_and_tables() {
    assert_eq!(
        command_of("+helm/dock tape 3 x", false),
        Some(("+helm".into(), vec!["dock".into()]))
    );
    assert_eq!(
        command_of("+tv/{n} {act}", false),
        Some(("+tv".into(), vec![]))
    );
    assert_eq!(
        command_of("DOCK TRANS UP", false),
        Some(("DOCK".into(), vec![]))
    );
    assert_eq!(
        command_of("HEADING {} QUIET", false),
        Some(("HEADING".into(), vec![]))
    );
    assert_eq!(command_of("look", false), None);
    assert_eq!(command_of("look", true), Some(("look".into(), vec![])));
    assert_eq!(command_of("Fire at will", true), None);
    assert_eq!(command_of("+", false), None);
    let source = "let a = \"FIRE HOLD\"; // \"DOCK X\"\n// \"SCAN\"\nRun(\"look\")\n";
    let found: Vec<String> = literals(source).into_iter().map(|(_, l)| l).collect();
    assert_eq!(found, ["FIRE HOLD", "look"]);
}
