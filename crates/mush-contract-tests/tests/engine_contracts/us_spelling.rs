//! **The game speaks American English** (owner ruling, 2026-11-17).
//!
//! British spellings had leaked into the words a player types and reads —
//! `+coop/enrol`, `@licence`, "the simulator centre" — because nothing checked
//! them. The commands moved to US spelling with the British forms kept as
//! aliases (`CommandSpec::aliases`, `CommandSpec::switch_aliases`), and this
//! gate stops a British spelling from coming back into:
//!
//! - every help page under `docs/help/`;
//! - every content pack's player text (`crates/*/content/**/*.json`): each
//!   string value with whitespace in it, except design notes and the keyword
//!   fields named in [`SKIPPED_KEYS`]; an exit's `names` is checked for its
//!   display name only, since the `;`-separated aliases after it are what a
//!   player may type and may keep an old spelling;
//! - every registered command's name and switches (aliases excepted);
//! - every prose string literal (one with whitespace in it) in production
//!   source, `crates/*/src/**/*.rs` outside `#[cfg(test)]`: the sentences the
//!   game prints, the mush-bot sentence table among them.
//!
//! The word list and the allowlist are `us-spelling.toml`, beside this crate's
//! `Cargo.toml`; its header says what the reader skips (identifiers, keys,
//! command tokens) and why. A failure names the file, the word and the US form.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mush_contract_tests::scan;
use serde::Deserialize;

use crate::production_registry::production_composition;

/// The checked-in word lists, beside this crate's `Cargo.toml`.
const LISTS: &str = "us-spelling.toml";

/// Content-pack keys whose values are not player text: design notes, and
/// keyword fields whose values are identifiers a pack's reader matches.
const SKIPPED_KEYS: &[&str] = &["note", "notes", "mode", "modes", "digest", "id", "kind"];

/// Content files that are not player text. A house mind's manifest is part of
/// the mind's hashed source and its `authority` field is the author's boundary
/// statement, not a sentence the game prints.
fn skipped_content(rel: &str) -> bool {
    rel.contains("/content/minds/") && rel.ends_with(".manifest.json")
}

// ---- the lists ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lists {
    ise: Ise,
    #[serde(default)]
    word: Vec<Word>,
    #[serde(default)]
    allow: Vec<Allow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ise {
    stems: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Word {
    british: String,
    us: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allow {
    file: String,
    word: String,
    context: String,
    reason: String,
}

fn lists() -> Lists {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(LISTS);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// British spelling (lowercase) to US spelling (lowercase).
#[derive(Debug, Default)]
struct Denylist(BTreeMap<String, String>);

impl Denylist {
    fn from_lists(lists: &Lists) -> Denylist {
        let mut map = BTreeMap::new();
        for w in &lists.word {
            assert!(
                map.insert(w.british.to_lowercase(), w.us.to_lowercase())
                    .is_none(),
                "{LISTS}: {} is listed twice",
                w.british
            );
        }
        for stem in &lists.ise.stems {
            let us_stem = format!("{}z", &stem[..stem.len() - 1]);
            for prefix in ["", "un", "re", "dis", "de"] {
                for ending in ["e", "es", "ed", "ing", "ation", "ations", "er", "ers"] {
                    if stem == "emphasis" && !matches!(ending, "e" | "es" | "ed" | "ing") {
                        continue;
                    }
                    map.insert(
                        format!("{prefix}{stem}{ending}"),
                        format!("{prefix}{us_stem}{ending}"),
                    );
                }
            }
        }
        Denylist(map)
    }

    fn us_of(&self, word: &str) -> Option<String> {
        self.0
            .get(&word.to_ascii_lowercase())
            .map(|us| case_like(word, us))
    }
}

/// `us` cased the way `word` is: `LICENCE` -> `LICENSE`, `Licence` -> `License`.
fn case_like(word: &str, us: &str) -> String {
    if word.len() > 1 && word.chars().all(|c| c.is_ascii_uppercase()) {
        return us.to_ascii_uppercase();
    }
    if word.starts_with(|c: char| c.is_ascii_uppercase()) {
        let mut chars = us.chars();
        return chars
            .next()
            .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
            .unwrap_or_default();
    }
    us.to_string()
}

// ---- the reader ------------------------------------------------------------

/// One British word, with the US form it should be.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Hit {
    /// Repo-relative path.
    file: String,
    /// Where in the file: a line number, or a pack's JSON path.
    at: String,
    /// The word as written.
    word: String,
    /// Its US spelling.
    us: String,
    /// The text around it, for the failure message and the allowlist.
    text: String,
}

/// Whether the word at `start..end` of `text` is an identifier or a command
/// token rather than prose (the `us-spelling.toml` header says why each).
fn skipped(text: &[u8], start: usize, end: usize) -> bool {
    let at = |i: usize| text.get(i).copied().unwrap_or(b' ');
    let prev = if start == 0 { b' ' } else { at(start - 1) };
    let next = at(end);
    let after = at(end + 1);
    let identish = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'{' | b'<' | b'_');
    matches!(prev, b':' | b'.' | b'_' | b'{' | b'"' | b'\'' | b'\\' | b'@' | b'+' | b'/')
        || matches!(next, b'_' | b'}' | b'"' | b'\\')
        || (matches!(next, b'.' | b':') && identish(after))
        || (prev == b'`' && next == b'`')
        || (next == b'/' && identish(after))
}

/// Every British word in a run of prose.
fn british_words(deny: &Denylist, text: &str) -> Vec<(String, String)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        // A whole word: not glued to a digit or an underscore on either side.
        let glued = |c: Option<&u8>| c.is_some_and(|c| c.is_ascii_digit() || *c == b'_');
        if glued(start.checked_sub(1).and_then(|p| bytes.get(p))) || glued(bytes.get(i)) {
            continue;
        }
        let word = &text[start..i];
        if let Some(us) = deny.us_of(word)
            && !skipped(bytes, start, i)
        {
            found.push((word.to_string(), us));
        }
    }
    found
}

/// One help page's British words, by line. The gate and its own test both read
/// pages through this.
fn help_page_hits(deny: &Denylist, file: &str, body: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (n, line) in body.lines().enumerate() {
        for (word, us) in british_words(deny, line) {
            hits.push(Hit {
                file: file.to_string(),
                at: (n + 1).to_string(),
                word,
                us,
                text: line.trim().to_string(),
            });
        }
    }
    hits
}

fn help_hits(deny: &Denylist) -> Vec<Hit> {
    let root = scan::repo_root();
    let pages = scan::files(&root.join("docs").join("help"), &["md"], &[]);
    assert!(pages.len() > 8, "the help directory did not load");
    let mut hits = Vec::new();
    for page in pages {
        let body = std::fs::read_to_string(&page).unwrap_or_else(|e| panic!("{page:?}: {e}"));
        hits.extend(help_page_hits(deny, &scan::relative(&root, &page), &body));
    }
    hits
}

/// One pack's British words in its player text.
fn pack_hits(deny: &Denylist, file: &str, value: &serde_json::Value) -> Vec<Hit> {
    fn walk(
        deny: &Denylist,
        file: &str,
        key: &str,
        path: &str,
        value: &serde_json::Value,
        hits: &mut Vec<Hit>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    walk(deny, file, k, &format!("{path}/{k}"), v, hits);
                }
            }
            serde_json::Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    walk(deny, file, key, &format!("{path}/{i}"), v, hits);
                }
            }
            serde_json::Value::String(text) => {
                if SKIPPED_KEYS.contains(&key) || !text.contains(char::is_whitespace) {
                    return;
                }
                // An exit's names: the display name, then typed aliases.
                let prose = if key == "names" {
                    text.split(';').next().unwrap_or_default()
                } else {
                    text.as_str()
                };
                for (word, us) in british_words(deny, prose) {
                    hits.push(Hit {
                        file: file.to_string(),
                        at: path.to_string(),
                        word,
                        us,
                        text: text.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    let mut hits = Vec::new();
    walk(deny, file, "", "", value, &mut hits);
    hits
}

fn content_hits(deny: &Denylist) -> Vec<Hit> {
    let root = scan::repo_root();
    let mut hits = Vec::new();
    let mut packs = 0_usize;
    for dir in scan::crate_dirs(&root) {
        for file in scan::files(&dir.join("content"), &["json"], &["target"]) {
            let rel = scan::relative(&root, &file);
            if skipped_content(&rel) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{rel}: {e}"));
            let value: serde_json::Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{rel}: {e}"));
            hits.extend(pack_hits(deny, &rel, &value));
            packs += 1;
        }
    }
    assert!(packs > 50, "the content packs did not load: {packs}");
    hits
}

/// A string literal's text: the quotes and any `b`/`r#` prefix off, and a
/// plain string's escapes read (a line continuation joins, `\n` and `\t` are a
/// space, `\"` is a quote), so the reader sees the words the game prints.
fn literal_text(literal: &str) -> String {
    let body = literal.strip_prefix('b').unwrap_or(literal);
    if let Some(raw) = body.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let inner = &raw[hashes..];
        let inner = inner.strip_prefix('"').unwrap_or(inner);
        let inner = inner
            .strip_suffix(&"#".repeat(hashes))
            .unwrap_or(inner)
            .strip_suffix('"')
            .unwrap_or(inner);
        return inner.to_string();
    }
    let inner = body.strip_prefix('"').unwrap_or(body);
    let inner = inner.strip_suffix('"').unwrap_or(inner);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\n' | '\r') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some('n' | 't' | 'r' | '0') => out.push(' '),
            Some(q @ ('"' | '\'' | '\\')) => out.push(q),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(u8::from_str_radix(&hex, 16).map_or(' ', char::from));
            }
            Some('u') => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
                out.push(' ');
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => {}
        }
    }
    out
}

/// Whether the literal starting at `start` sits inside a `#[cfg(test)]` item:
/// the first code byte before it was blanked by the test-item pass.
fn in_test_item(stripped: &[u8], production: &[u8], start: usize) -> bool {
    let mut i = start;
    while i > 0 {
        i -= 1;
        if !stripped[i].is_ascii_whitespace() {
            return production[i] != stripped[i];
        }
    }
    false
}

/// One production source file's British words in its prose literals.
fn source_file_hits(deny: &Denylist, file: &str, src: &str) -> Vec<Hit> {
    let stripped = scan::strip_comments_and_literals(src);
    if scan::file_is_test_only(&stripped) {
        return Vec::new();
    }
    let production = scan::blank_cfg_test_items(&stripped);
    let mut hits = Vec::new();
    for span in scan::string_literals(src) {
        if in_test_item(stripped.as_bytes(), production.as_bytes(), span.start) {
            continue;
        }
        let text = literal_text(&src[span.clone()]);
        if !text.contains(char::is_whitespace) {
            continue;
        }
        let line = src[..span.start].matches('\n').count() + 1;
        for (word, us) in british_words(deny, &text) {
            hits.push(Hit {
                file: file.to_string(),
                at: line.to_string(),
                word,
                us,
                text: text.trim().to_string(),
            });
        }
    }
    hits
}

fn source_hits(deny: &Denylist) -> Vec<Hit> {
    let root = scan::repo_root();
    let mut hits = Vec::new();
    let mut read = 0_usize;
    for dir in scan::crate_dirs(&root) {
        for file in scan::files(&dir.join("src"), &["rs"], &["tests", "target"]) {
            let rel = scan::relative(&root, &file);
            let src = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{rel}: {e}"));
            hits.extend(source_file_hits(deny, &rel, &src));
            read += 1;
        }
    }
    assert!(read > 500, "the source tree did not load: {read} files");
    hits
}

/// The registered command names and switches. An alias is exempt: it is how a
/// British spelling keeps working.
fn command_hits(deny: &Denylist) -> Vec<Hit> {
    let composition = production_composition();
    let mut hits = Vec::new();
    for spec in composition.registry().specs() {
        let words = std::iter::once(spec.name.as_str()).chain(spec.switches.iter().map(|s| s.as_str()));
        for written in words {
            for word in written
                .split(|c: char| !c.is_ascii_alphabetic())
                .filter(|w| !w.is_empty())
            {
                if let Some(us) = deny.us_of(word) {
                    hits.push(Hit {
                        file: "the production command registry".to_string(),
                        at: spec.name.to_string(),
                        word: word.to_string(),
                        us,
                        text: written.to_string(),
                    });
                }
            }
        }
    }
    hits
}

// ---- the gate ----------------------------------------------------------------

#[test]
fn player_facing_text_is_in_us_spelling() {
    let lists = lists();
    let deny = Denylist::from_lists(&lists);
    let mut hits = help_hits(&deny);
    hits.extend(content_hits(&deny));
    hits.extend(command_hits(&deny));
    hits.extend(source_hits(&deny));

    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    for hit in &hits {
        let allowed = lists.allow.iter().position(|row| {
            row.file == hit.file && row.word == hit.word && hit.text.contains(&row.context)
        });
        if let Some(row) = allowed {
            used.insert(row);
            continue;
        }
        failures.push(format!(
            "{}:{}: \"{}\" is British spelling; write \"{}\" (in: {})",
            hit.file,
            hit.at,
            hit.word,
            hit.us,
            hit.text.chars().take(120).collect::<String>()
        ));
    }
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} British spelling(s) in player-facing text. Write the US form; a proper \
         noun or a quoted in-world name may take an [[allow]] row in {LISTS} with \
         a reason:\n{}",
        failures.len(),
        failures.join("\n")
    );

    for (i, row) in lists.allow.iter().enumerate() {
        assert!(
            !row.reason.trim().is_empty(),
            "{LISTS}: the allow row for {} in {} needs a reason",
            row.word,
            row.file
        );
        assert!(
            used.contains(&i),
            "{LISTS}: the allow row for {} in {} matches no text any more; drop it",
            row.word,
            row.file
        );
    }
}

// ---- the reader's own table --------------------------------------------------

fn test_denylist() -> Denylist {
    Denylist::from_lists(&lists())
}

/// The gate's proof: a help page with a planted "enrol" fails it, through the
/// same reader the gate runs on every page.
#[test]
fn a_planted_enrol_in_a_help_page_is_caught() {
    let deny = test_denylist();
    let page = "\
# Co-op

Sign with `+coop/enroll <name>`, then enrol your crew.
";
    let hits = help_page_hits(&deny, "docs/help/planted.md", page);
    assert_eq!(hits.len(), 1, "{hits:#?}");
    assert_eq!(hits[0].file, "docs/help/planted.md");
    assert_eq!(hits[0].at, "3");
    assert_eq!(hits[0].word, "enrol");
    assert_eq!(hits[0].us, "enroll");
}

#[test]
fn the_reader_keeps_case_and_skips_identifiers_and_command_tokens() {
    let deny = test_denylist();
    let words = |text: &str| british_words(&deny, text);
    assert_eq!(
        words("The Licence and the ORGANISATION were catalogued."),
        vec![
            ("Licence".to_string(), "License".to_string()),
            ("ORGANISATION".to_string(), "ORGANIZATION".to_string()),
            ("catalogued".to_string(), "cataloged".to_string()),
        ]
    );
    assert_eq!(
        words("organised, unrecognised and emphasised; emphasis is US spelling"),
        vec![
            ("organised".to_string(), "organized".to_string()),
            ("unrecognised".to_string(), "unrecognized".to_string()),
            ("emphasised".to_string(), "emphasized".to_string()),
        ]
    );
    for identifier in [
        "the tacsim:licence: document",
        "seal filed:{licence} aboard",
        "fact minds licence.lapsed now",
        "reference licence/<16-digit-id> here",
        r#"{"licence":7} as JSON"#,
        "the `offence` mode",
        "type +licence/buy or @licence/show or +coop/enrol",
        "licence_id and licence2 are code",
    ] {
        assert!(words(identifier).is_empty(), "{identifier}: {:?}", words(identifier));
    }
    assert_eq!(
        words("your licence."),
        vec![("licence".to_string(), "license".to_string())],
        "a full stop ending a sentence is not a dotted identifier"
    );
}

#[test]
fn the_source_reader_reads_prose_literals_outside_tests_only() {
    let deny = test_denylist();
    let src = r##"
pub const SAID: &str = "Your licence \
    lapsed.\nRenew at the centre.";
pub const KEY: &str = "tacsim:licence:";
pub const WORD: &str = "licence";
// "a comment about the licence centre"
const RAW: &str = r#"the "grey" cargo"#;

#[cfg(test)]
mod tests {
    const PLANTED: &str = "a test may say enrol";
}
"##;
    let hits: Vec<(String, String)> = source_file_hits(&deny, "x.rs", src)
        .into_iter()
        .map(|h| (h.at, h.word))
        .collect();
    assert_eq!(
        hits,
        vec![
            ("2".to_string(), "licence".to_string()),
            ("2".to_string(), "centre".to_string()),
            ("7".to_string(), "grey".to_string()),
        ],
        "a key, a one-word literal, a comment and a test item are not prose the game prints"
    );
}

#[test]
fn a_packs_reader_skips_notes_keywords_and_exit_aliases() {
    let deny = test_denylist();
    let pack = serde_json::json!({
        "entries": {
            "room.office": {"def": {
                "desc": "The harbourmaster's grey desk.",
                "note": "Design note: the defence mode is provisional.",
            }},
            "exit.office": {"def": {"names": ["Harbormaster <HM>;hm;harbourmaster;harbour"]}},
            "mount": {"def": {"modes": ["offence", "defence"]}},
        }
    });
    let hits: Vec<(String, String)> = pack_hits(&deny, "pack.json", &pack)
        .into_iter()
        .map(|h| (h.at, h.word))
        .collect();
    assert_eq!(
        hits,
        vec![
            ("/entries/room.office/def/desc".to_string(), "harbourmaster".to_string()),
            ("/entries/room.office/def/desc".to_string(), "grey".to_string()),
        ]
    );
}
