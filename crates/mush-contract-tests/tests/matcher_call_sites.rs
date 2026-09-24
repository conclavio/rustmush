//! **The matcher's call sites, audited** (16b cut U.4; 16c `T-67`, answering
//! `Q34` at its default: *the gate and `@find` now; the lens is H6*).
//!
//! ## What the finding was
//!
//! `mush_core::matchit::match_object` resolves `#N` for any object in the world
//! and `*name` for any player, with no observer test — and every command that
//! names a target goes through it. Doc 16 §3.6's world-side exclusion (*nothing
//! an arena holds may be found, named or sighted*) reached `look`, `@find`,
//! `+map` and the unit listings one surface at a time, but the kernel matcher
//! has no perception lens at all, and 16b cut S's residual (14) recorded that a
//! staffer who already knows a deployed tank's name still resolves it. The lens
//! itself — the matcher asking `can_perceive` — is H6's work. This is what comes
//! first: **a list of every place that calls it, each with what guards it**, so
//! the lens has a work list and a new call site cannot be written without
//! somebody saying which guard it has.
//!
//! ## The scan
//!
//! Every `crates/*/src/**/*.rs` file (never `tests/`, which are separate
//! crates — the boundary the panic budget and the restriction lints draw) is
//! stripped of `//` line comments, `///` and `//!` doc comments, block comments
//! and string and character literals, and then of every `#[cfg(test)]` item,
//! brace-counted from the attribute to its close. What is left is searched for
//! `match_object(` as a whole word. `crates/mush-core/src/matchit.rs` is skipped
//! whole: it is the definition, and its own recursion and unit tests are not
//! call sites.
//!
//! Each hit is attributed to its **enclosing `fn`**, qualified by the `impl` (or
//! `trait`) block it is a method of — `Type::method`, or the bare function name —
//! so two same-named methods of two types in one file are two rows. A closure is
//! its enclosing function's: a command registered as `Arc::new(|ctx| …)` inside
//! `register` is `register`'s row, and its note names the verb.
//!
//! Every `(file, function)` the scan finds needs exactly one `[[site]]` row in
//! `matcher-call-sites.toml`, and every row must still name a site the scan
//! finds — an unused row fails, which is the help allowlist's rule — and every
//! row's `guard` is one of the five words that file's head defines.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The checked-in audit, beside this crate's `Cargo.toml`.
const PIN: &str = "matcher-call-sites.toml";

/// The call the scan counts.
const NEEDLE: &str = "match_object(";

/// The definition's own file, skipped whole (relative to `crates/`).
const DEFINITION: &str = "mush-core/src/matchit.rs";

/// The five guard words, defined in the pin's head.
const GUARDS: [&str; 5] = ["perceive", "locality", "staff", "self", "none"];

/// **The census floor**: the scan must find at least this many `(file,
/// function)` sites. Without it a scanner that silently stopped finding anything
/// would pass against a pin that had been emptied to match. At 16b cut U.4 the
/// scan found 110; lower this in the same diff that removes sites, never to make
/// a broken scan pass.
const CENSUS_FLOOR: usize = 110;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    site: Vec<SiteRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteRow {
    /// The file, relative to `crates/`, with forward slashes.
    file: String,
    /// `function`, or `Type::method`.
    function: String,
    /// One of [`GUARDS`].
    guard: String,
    /// Why, when the guard word alone does not say it.
    #[serde(default)]
    note: Option<String>,
}

// ---------------------------------------------------------------------------
// The source walk
// ---------------------------------------------------------------------------

/// The workspace's `crates/` directory.
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/<pkg> has a crates/ parent")
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

/// Every production source: `crates/*/src/**/*.rs`, keyed by its path relative
/// to `crates/` with forward slashes, the definition excluded.
fn production_sources(crates: &Path) -> Vec<(String, String)> {
    let entries = fs::read_dir(crates)
        .unwrap_or_else(|err| panic!("{} is not readable: {err}", crates.display()));
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    dirs.sort();
    let mut sources = Vec::new();
    for dir in dirs {
        if !dir.join("Cargo.toml").is_file() {
            continue;
        }
        for path in rust_files(&dir.join("src")) {
            let relative = path
                .strip_prefix(crates)
                .unwrap_or(&path)
                .display()
                .to_string()
                .replace('\\', "/");
            if relative == DEFINITION {
                continue;
            }
            let text =
                fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            sources.push((relative, text));
        }
    }
    sources
}

// ---------------------------------------------------------------------------
// Stripping (the panic budget's own two passes, carried here as its rig is)
// ---------------------------------------------------------------------------

/// Blank every comment and every string or character literal, keeping each
/// newline so line numbers survive. `//`, `///` and `//!` all run to the end of
/// the line; block comments nest.
fn strip_comments_and_literals(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let len = bytes.len();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for slot in &mut out[from..to] {
            if *slot != b'\n' {
                *slot = b' ';
            }
        }
    };
    while i < len {
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            blank(&mut out, start, i);
            continue;
        }
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            let mut depth = 0usize;
            while i < len {
                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                i += 1;
            }
            blank(&mut out, start, i.min(len));
            continue;
        }
        let ident_before = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        // `b"…"`, `b'c'`, `br"…"`: blank the prefix and let the arms below take
        // the literal itself.
        if !ident_before
            && bytes[i] == b'b'
            && i + 1 < len
            && (bytes[i + 1] == b'"'
                || bytes[i + 1] == b'\''
                || (bytes[i + 1] == b'r'
                    && i + 2 < len
                    && (bytes[i + 2] == b'"' || bytes[i + 2] == b'#')))
        {
            out[i] = b' ';
            i += 1;
            continue;
        }
        // A raw string has no escapes and is closed by `"` plus its hashes.
        if !ident_before
            && bytes[i] == b'r'
            && i + 1 < len
            && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#')
        {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < len && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                let mut terminator = String::from('"');
                for _ in 0..hashes {
                    terminator.push('#');
                }
                let body = j + 1;
                let end = src[body..]
                    .find(&terminator)
                    .map_or(len, |rel| body + rel + terminator.len());
                blank(&mut out, i, end);
                i = end;
                continue;
            }
        }
        // An ordinary string, whose escapes may hide the closing quote.
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                let closing = bytes[i] == b'"';
                i += 1;
                if closing {
                    break;
                }
            }
            blank(&mut out, start, i.min(len));
            continue;
        }
        // A quote is a character literal only when one character later comes
        // its twin; otherwise it opens a lifetime and is ordinary code.
        if bytes[i] == b'\'' {
            if i + 1 < len && bytes[i + 1] == b'\\' {
                let mut j = i + 2;
                while j < len && bytes[j] != b'\'' {
                    j += 1;
                }
                let end = (j + 1).min(len);
                blank(&mut out, i, end);
                i = end;
                continue;
            }
            if let Some(c) = src[i + 1..].chars().next() {
                let after = i + 1 + c.len_utf8();
                if after < len && bytes[after] == b'\'' {
                    blank(&mut out, i, after + 1);
                    i = after + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    String::from_utf8(out).expect("blanking whole byte ranges leaves valid UTF-8")
}

/// The attribute that puts an item out of a production build.
const CFG_TEST: &str = "#[cfg(test)]";

/// Blank each `#[cfg(test)]` item, brace-counted from the attribute to its
/// matching close — a `mod tests { … }` at the end of a file, or one in the
/// middle of it. An attribute whose item is not brace-delimited (`#[cfg(test)]
/// use super::*;`) reaches its `;` first and is left alone. Input must already be
/// stripped, so a brace inside a string cannot throw the depth off.
fn blank_cfg_test_items(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut search = 0;
    while let Some(rel) = src[search..].find(CFG_TEST) {
        let at = search + rel;
        let mut cursor = at + CFG_TEST.len();
        let mut open = None;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'{' => {
                    open = Some(cursor);
                    break;
                }
                b';' => break,
                _ => cursor += 1,
            }
        }
        let Some(open) = open else {
            search = at + CFG_TEST.len();
            continue;
        };
        let mut depth = 0usize;
        let mut end = open;
        while end < bytes.len() {
            match bytes[end] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        let stop = end.min(bytes.len().saturating_sub(1));
        for slot in &mut out[at..=stop] {
            if *slot != b'\n' {
                *slot = b' ';
            }
        }
        search = stop + 1;
    }
    String::from_utf8(out).expect("blanking whole byte ranges leaves valid UTF-8")
}

// ---------------------------------------------------------------------------
// The scanner
// ---------------------------------------------------------------------------

/// What a brace opened, as far as attribution cares.
#[derive(Clone, Debug)]
enum Scope {
    /// A function body.
    Fn(String),
    /// An `impl` block, by its self type's last path segment.
    Impl(String),
    /// A `trait` block, by its name.
    Trait(String),
    /// Anything else: a block, a module, a struct, a match arm.
    Other,
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The identifier starting at `at`, if one does.
fn ident_at(text: &str, at: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = at;
    let mut end = at;
    while end < bytes.len() && is_ident(bytes[end]) {
        end += 1;
    }
    (end > start && !bytes[start].is_ascii_digit()).then(|| &text[start..end])
}

/// Skip whitespace from `at`.
fn skip_ws(text: &str, mut at: usize) -> usize {
    let bytes = text.as_bytes();
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// The self type of an `impl` head (the text between `impl` and its `{`):
/// generics dropped, a `where` clause cut, the type after `for` when it is a
/// trait impl, and the last path segment of what is left.
fn impl_self_type(head: &str) -> String {
    let mut head = head.trim();
    if head.starts_with('<') {
        let mut depth = 0usize;
        let mut cut = head.len();
        for (offset, ch) in head.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        cut = offset + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        head = head[cut..].trim();
    }
    // The words at angle depth zero, so `Foo<for<'a> Fn(&'a X)>` is not split.
    let mut words: Vec<(usize, &str)> = Vec::new();
    let mut depth = 0usize;
    let bytes = head.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => depth = depth.saturating_sub(1),
            _ if depth == 0 && is_ident(bytes[i]) && (i == 0 || !is_ident(bytes[i - 1])) => {
                if let Some(word) = ident_at(head, i) {
                    words.push((i, word));
                    i += word.len();
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let mut end = head.len();
    if let Some((at, _)) = words.iter().find(|(_, word)| *word == "where") {
        end = *at;
    }
    let mut start = 0;
    if let Some((at, _)) = words.iter().find(|(at, word)| *word == "for" && *at < end) {
        start = at + 3;
    }
    let ty = head[start..end].trim().trim_start_matches('&').trim();
    let ty = ty.strip_prefix("dyn ").unwrap_or(ty).trim();
    let path = ty.split('<').next().unwrap_or(ty).trim();
    path.rsplit("::").next().unwrap_or(path).trim().to_owned()
}

/// **Every `match_object(` in one stripped source, as `(line, function)`.**
///
/// A brace-depth walk: an `fn name`, an `impl … ` head or a `trait Name` opens
/// a pending scope that the next `{` at bracket depth zero pushes (and a `;`
/// there cancels, which is a body-less trait method); every other `{` pushes an
/// anonymous scope; every `}` pops. `impl` is only a block head when no `fn` is
/// pending, so `-> impl Iterator` and `x: impl Fn()` are never taken for one.
fn call_sites(stripped: &str) -> Vec<(usize, String)> {
    let bytes = stripped.as_bytes();
    let mut stack: Vec<Scope> = Vec::new();
    let mut pending: Option<Scope> = None;
    // Parens and square brackets since the pending head began: a `{` or a `;`
    // inside them (a const generic, an array length) is not the head's end.
    let mut nesting = 0usize;
    let mut impl_head_from: Option<usize> = None;
    let mut sites = Vec::new();
    let mut line = 1usize;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if is_ident(byte) && (i == 0 || !is_ident(bytes[i - 1])) {
            let Some(word) = ident_at(stripped, i) else {
                i += 1;
                continue;
            };
            let after = i + word.len();
            if word == "match_object" && bytes.get(after) == Some(&b'(') {
                sites.push((line, attribute(&stack)));
            } else if word == "fn" && pending.is_none() {
                let name_at = skip_ws(stripped, after);
                if name_at > after
                    && let Some(name) = ident_at(stripped, name_at)
                {
                    pending = Some(Scope::Fn(name.to_owned()));
                    nesting = 0;
                }
            } else if word == "impl" && pending.is_none() {
                pending = Some(Scope::Impl(String::new()));
                impl_head_from = Some(after);
                nesting = 0;
            } else if word == "trait" && pending.is_none() {
                let name_at = skip_ws(stripped, after);
                if let Some(name) = ident_at(stripped, name_at) {
                    pending = Some(Scope::Trait(name.to_owned()));
                    nesting = 0;
                }
            }
            i = after;
            continue;
        }
        match byte {
            b'(' | b'[' if pending.is_some() => nesting += 1,
            b')' | b']' if pending.is_some() => nesting = nesting.saturating_sub(1),
            b';' if pending.is_some() && nesting == 0 => {
                pending = None;
                impl_head_from = None;
            }
            b'{' => {
                let scope = match pending.take() {
                    Some(_) if nesting > 0 => Scope::Other,
                    Some(Scope::Impl(_)) => Scope::Impl(
                        impl_head_from
                            .map(|from| impl_self_type(&stripped[from..i]))
                            .unwrap_or_default(),
                    ),
                    Some(scope) => scope,
                    None => Scope::Other,
                };
                impl_head_from = None;
                stack.push(scope);
            }
            b'}' => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    sites
}

/// The innermost enclosing function, qualified by the nearest `impl` or `trait`
/// block around it.
fn attribute(stack: &[Scope]) -> String {
    let Some(at) = stack
        .iter()
        .rposition(|scope| matches!(scope, Scope::Fn(_)))
    else {
        return "<no fn>".to_owned();
    };
    let Scope::Fn(name) = &stack[at] else {
        return "<no fn>".to_owned();
    };
    let owner = stack[..at].iter().rev().find_map(|scope| match scope {
        Scope::Impl(ty) | Scope::Trait(ty) => Some(ty.clone()),
        _ => None,
    });
    match owner {
        Some(owner) if !owner.is_empty() => format!("{owner}::{name}"),
        _ => name.clone(),
    }
}

/// The whole scan over a set of sources: `(file, function) → hit lines`.
fn census(sources: &[(String, String)]) -> BTreeMap<(String, String), Vec<usize>> {
    let mut found: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (file, text) in sources {
        if !text.contains(NEEDLE) {
            continue;
        }
        let stripped = blank_cfg_test_items(&strip_comments_and_literals(text));
        for (line, function) in call_sites(&stripped) {
            found
                .entry((file.clone(), function))
                .or_default()
                .push(line);
        }
    }
    found
}

fn read_pin() -> Pin {
    let path = crates_dir().join("mush-contract-tests").join(PIN);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    toml::from_str(&text).unwrap_or_else(|err| panic!("{} does not parse: {err}", path.display()))
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

#[test]
fn every_matcher_call_site_is_listed_with_its_guard() {
    let found = census(&production_sources(&crates_dir()));
    assert!(
        found.len() >= CENSUS_FLOOR,
        "the scan found {} `{NEEDLE}` sites, under the floor of {CENSUS_FLOOR} — a scan that \
         stopped finding anything would pass against a pin emptied to match it",
        found.len()
    );

    let pin = read_pin();
    let mut complaints = Vec::new();
    let mut listed: BTreeSet<(String, String)> = BTreeSet::new();
    for row in &pin.site {
        if !GUARDS.contains(&row.guard.as_str()) {
            complaints.push(format!(
                "  {} {}: guard `{}` is not one of {GUARDS:?} — {PIN}'s head defines them",
                row.file, row.function, row.guard
            ));
        }
        if row.function.trim().is_empty() || row.file.trim().is_empty() {
            complaints.push(format!("  a row with an empty file or function: {row:?}"));
        }
        if row
            .note
            .as_deref()
            .is_some_and(|note| note.trim().is_empty())
        {
            complaints.push(format!(
                "  {} {}: an empty note — leave it out instead",
                row.file, row.function
            ));
        }
        if !listed.insert((row.file.clone(), row.function.clone())) {
            complaints.push(format!(
                "  {} {}: listed twice; one row per (file, function)",
                row.file, row.function
            ));
        }
    }
    for ((file, function), lines) in &found {
        if !listed.contains(&(file.clone(), function.clone())) {
            complaints.push(format!(
                "  {file} {function} (line {lines:?}): calls `{NEEDLE}` and is not in {PIN}. \
                 Add a [[site]] row naming its guard — perceive, locality, staff, self or \
                 none — and read the head of {PIN} first: `locality` and `staff` do not \
                 satisfy doc 16 §3.6's exclusion."
            ));
        }
    }
    for (file, function) in &listed {
        if !found.contains_key(&(file.clone(), function.clone())) {
            complaints.push(format!(
                "  {file} {function}: {PIN} lists it and the scan no longer finds a `{NEEDLE}` \
                 there. Delete the row (or rename it with the function)."
            ));
        }
    }
    assert!(
        complaints.is_empty(),
        "the matcher audit ({PIN}) no longer matches the tree:\n{}",
        complaints.join("\n")
    );
}

/// **The scanner, on a source written to trip it** — every part of the scan
/// together (the stripping, the `#[cfg(test)]` blanking and the attribution),
/// because exercising one piece alone is how a hole gets through.
#[test]
fn the_scanner_finds_calls_qualifies_methods_and_skips_comments_literals_and_tests() {
    let source = r#"
use crate::matchit::match_object;

/// A doc comment naming match_object(w, a, b) is not a call.
//! Nor is an inner one: match_object(w, a, b).
fn free(w: &World) {
    // match_object(w, a, "in a comment")
    let _ = "match_object( in a string";
    let _ = match_object(w, a, b);
}

impl<'a> Ctx<'a> {
    pub fn resolve(&self) -> impl Iterator<Item = u8> + '_ {
        let hit = |raw: &str| match_object(self.w, self.actor, raw);
        hit("x");
        std::iter::empty()
    }
}

impl EventHook for PlacementGate {
    fn check(&self, w: &World) {
        match_object(w, a, b).ok();
    }
}

impl DestroyGate for StructuralGate {
    fn check(&self, w: &World) {
        fn inner(w: &World, a: [u8; 2]) {
            match_object(w, a, b).ok();
        }
    }
}

trait Finder {
    fn declared(&self);
    fn find(&self, w: &World) {
        let _ = crate::matchit::match_object(w, a, b);
    }
}

fn not_a_call() {
    try_match_object(w, a, b);
    let match_object_count = 3;
}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        match_object(w, a, b).unwrap();
    }
}

fn after_the_tests(w: &World) {
    match_object(w, a, b);
}
"#;
    let found = census(&[("synthetic/src/lib.rs".to_owned(), source.to_owned())]);
    let functions: Vec<&str> = found
        .keys()
        .map(|(_, function)| function.as_str())
        .collect();
    assert_eq!(
        functions,
        vec![
            "Ctx::resolve",
            "Finder::find",
            "PlacementGate::check",
            "StructuralGate::inner",
            "after_the_tests",
            "free",
        ],
        "{found:?}"
    );
    let free = found
        .get(&("synthetic/src/lib.rs".to_owned(), "free".to_owned()))
        .expect("the free function's call");
    assert_eq!(
        free.len(),
        1,
        "the comment, the doc comments and the string are not calls: {free:?}"
    );
}
