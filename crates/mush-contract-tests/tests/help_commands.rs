//! **16c `T-45`: the help tells the truth about the command table** (16b cut
//! D.5).
//!
//! `docs/help/*.md` is prose, loaded at boot and served by `help <topic>`, and
//! nothing ever checked it against the verbs the server registers. That is how
//! three pages for deleted game rulesets, a worked example built on four deleted
//! verbs and a `FIRE mg` that did not exist all stayed green for four days of
//! cuts: a help page is the one surface in this tree with no compiler and no
//! test behind it.
//!
//! This is that test. It takes every **command-shaped token** out of the help —
//! the first word of each inline-code span and of each line inside a fenced
//! block, kept when it starts with `@` or `+`, or is written in capitals and is
//! at least three letters long — and demands that each one resolve against the
//! production registry, exactly and case-folded, or carry a row in
//! `help-commands-allowlist.toml` saying why it is not a verb.
//!
//! The allowlist needs a reason and nothing else, and an **unused** row fails
//! the test, so it cannot rot into a list of things that used to be wrong.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The checked-in allowlist, beside this crate's `Cargo.toml`.
const ALLOWLIST: &str = "help-commands-allowlist.toml";

/// The help directory, at the root of the workspace.
fn help_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("help")
}

mod production_registry;

use production_registry::production_composition;

// ---- the tokenizer -----------------------------------------------------------

/// One command-shaped token, with where it was read.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Token {
    /// The help file's name (`cockpit.md`).
    file: String,
    /// The verb, as written.
    word: String,
    /// The `/switch`es that followed it, in order. A command may take more than
    /// one (`@spatial/place/interior`), and the help writes them that way.
    switches: Vec<String>,
}

/// Trim the punctuation prose wraps a verb in, from both ends. `@`/`+` are
/// leading verb characters and are never trimmed off the front.
fn trim_word(word: &str) -> &str {
    let trailing = |c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/'));
    let leading = |c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '+'));
    word.trim_start_matches(leading).trim_end_matches(trailing)
}

/// Whether a trimmed word is command-shaped: an `@`/`+` verb, or a word written
/// in capitals with at least three letters in it (`SIM`, `BOARD`,
/// `WEAPONSTATUS`).
fn is_command_shaped(word: &str) -> bool {
    if word.starts_with('@') || word.starts_with('+') {
        return word.len() > 1;
    }
    let letters = word.chars().filter(char::is_ascii_alphabetic).count();
    letters >= 3
        && word
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// The command-shaped token at the head of `line`, if any.
fn head_token(file: &str, line: &str) -> Option<Token> {
    let word = trim_word(line.split_whitespace().next()?);
    if !is_command_shaped(word) {
        return None;
    }
    // Every `/switch` is checked against the command's own registered set. The
    // help writes optional ones in brackets (`@spatial/unplace[/keep]`), so a
    // segment is kept only as far as its first non-word character.
    let mut segments = word.split('/');
    let word = segments.next().unwrap_or_default().to_string();
    let switches: Vec<String> = segments
        .map(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect();
    if word.is_empty() {
        return None;
    }
    Some(Token {
        file: file.to_string(),
        word,
        switches,
    })
}

/// Every command-shaped token in one help page: the first word of each line
/// inside a fenced block, and the first word of each inline-code span.
fn tokens_of(file: &str, body: &str) -> Vec<Token> {
    let mut found = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        // A fence **delimiter** opens or closes a block: three backticks and at
        // most an info string. A line carrying a SECOND run of three backticks
        // is an inline span — `cockpit.md` writes one to quote a refusal that
        // itself contains backticks — and toggling on it would leave the
        // tokenizer fenced for the rest of the page, reading only head words
        // and silently skipping every inline span below it. That is exactly the
        // blind spot this test exists to close, so the delimiter is recognised
        // by the whole line and not by its first three characters.
        let trimmed = line.trim();
        if trimmed.starts_with("```") && trimmed.matches("```").count() == 1 {
            fenced = !fenced;
            continue;
        }
        if fenced {
            found.extend(head_token(file, line));
            continue;
        }
        // Inline spans: every run between backticks, alternate segments.
        for (i, span) in line.split('`').enumerate() {
            if i % 2 == 1 {
                found.extend(head_token(file, span));
            }
        }
    }
    found
}

// ---- the allowlist -----------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    #[serde(default)]
    token: Vec<AllowedToken>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowedToken {
    /// The token exactly as the help writes it.
    word: String,
    /// One line on why it is not a verb.
    reason: String,
}

fn allowlist() -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let parsed: Allowlist =
        toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut rows = BTreeMap::new();
    for row in parsed.token {
        assert!(
            !row.reason.trim().is_empty(),
            "{}: {} needs a reason",
            path.display(),
            row.word
        );
        assert!(
            rows.insert(row.word.clone(), row.reason).is_none(),
            "{}: {} is listed twice",
            path.display(),
            row.word
        );
    }
    rows
}

// ---- the gate ----------------------------------------------------------------

#[test]
fn every_command_shaped_token_in_the_help_is_a_registered_verb() {
    let composition = production_composition();
    let registry = composition.registry();
    let allowed = allowlist();

    let mut pages: Vec<(String, String)> = Vec::new();
    let dir = help_dir();
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("a help directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        pages.push((name, body));
    }
    assert!(pages.len() > 8, "the help directory did not load: {dir:?}");

    let mut unknown: Vec<String> = Vec::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (file, body) in &pages {
        for token in tokens_of(file, body) {
            if allowed.contains_key(&token.word) {
                used.insert(token.word.clone());
                continue;
            }
            let Some(spec) = registry.spec_of(&token.word) else {
                unknown.push(format!(
                    "{}: `{}` is no registered verb{}",
                    token.file,
                    token.word,
                    nearest(registry, &token.word)
                ));
                continue;
            };
            for switch in &token.switches {
                if spec.switches.iter().any(|s| s.eq_ignore_ascii_case(switch)) {
                    continue;
                }
                unknown.push(format!(
                    "{}: `{}` has no switch /{switch}; it takes {}",
                    token.file,
                    token.word,
                    if spec.switches.is_empty() {
                        "none".to_string()
                    } else {
                        spec.switches
                            .iter()
                            .map(|s| format!("/{s}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    }
                ));
            }
        }
    }
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "the help names {} thing(s) the server does not register. Fix the page, \
         or add a row to {ALLOWLIST} with a reason:\n{}",
        unknown.len(),
        unknown.join("\n")
    );

    let unused: Vec<&String> = allowed.keys().filter(|w| !used.contains(*w)).collect();
    assert!(
        unused.is_empty(),
        "{ALLOWLIST} carries {} row(s) no help page writes any more; drop them:\n{unused:#?}",
        unused.len()
    );
}

/// The nearest registered verb, for the failure message.
fn nearest(registry: &mush_core::cmd::Registry, word: &str) -> String {
    let folded = word.to_lowercase();
    let best = registry
        .specs()
        .map(|spec| spec.name.to_lowercase())
        .filter(|name| {
            name.starts_with(folded.get(..2).unwrap_or(&folded))
                || folded.starts_with(name.get(..2).unwrap_or(name))
        })
        .min_by_key(|name| name.len().abs_diff(folded.len()));
    best.map_or_else(String::new, |name| format!(" (nearest: `{name}`)"))
}

// ---- the tokenizer's own table ----------------------------------------------

#[test]
fn the_tokenizer_reads_fences_spans_switches_and_placeholders() {
    let sample = "\
Prose about `@tui` and `LOCK <contact>` and `+intel/clear`.
An optional switch reads `@spatial/unplace[/keep] <thing>`.
A `BA` is two letters, and `Power online.` is not capitals.

```
  STARTUP             bring the power up
  FIRE [<weapon>]     shoot the lock
  just prose in a fence
```

A refusal quoted whole, opening its line the way a fence would:
``` `m` could be machine_gun or missile; say more. ```
Back outside the fence: `SIM STATUS` and `<contact>`.
";
    let tokens: Vec<(String, Vec<String>)> = tokens_of("sample.md", sample)
        .into_iter()
        .map(|t| (t.word, t.switches))
        .collect();
    let plain = |w: &str| (w.to_string(), Vec::new());
    assert_eq!(
        tokens,
        vec![
            plain("@tui"),
            plain("LOCK"),
            ("+intel".to_string(), vec!["clear".to_string()]),
            (
                "@spatial".to_string(),
                vec!["unplace".to_string(), "keep".to_string()]
            ),
            plain("STARTUP"),
            plain("FIRE"),
            plain("SIM"),
        ],
        "the fence's prose line, the two-letter designator, the sentence-case \
         line and the bare placeholder are all skipped — and the inline span \
         that opens its line with three backticks does NOT open a fence, or the \
         `SIM` below it would never be read"
    );
}

// ---- the first-time hints ----------------------------------------------------

/// **16b cut AH.1 (16c N18; 16c section 6: "N18's hints quote the registered
/// table")**: every verb a hint names is a verb the production composition
/// registers, so a hint can never tell a first-time pilot to type a word the
/// server answers with "Huh?".
///
/// Each of `tacsim::match_::tutorial::HINT_LINES` is split on whitespace and
/// read with this file's own `trim_word` and `is_command_shaped`; every kept
/// word resolves through the registry — except the word directly after `SIM`,
/// which is one of `SIM`'s selectors and must be one of its registered switches,
/// case-folded (`TEAM` is `team`). No allowlist row: `Hint:` and `<contact>` are
/// not command-shaped.
#[test]
fn every_verb_a_hint_names_is_a_registered_verb() {
    let composition = production_composition();
    let registry = composition.registry();
    let sim = registry
        .spec_of("SIM")
        .expect("the production composition registers SIM");
    let mut checked = 0_usize;
    for line in tacsim::match_::tutorial::HINT_LINES {
        let mut after_sim = false;
        for raw in line.split_whitespace() {
            let word = trim_word(raw);
            if !is_command_shaped(word) {
                after_sim = false;
                continue;
            }
            if after_sim {
                assert!(
                    sim.switches.iter().any(|s| s.eq_ignore_ascii_case(word)),
                    "{line}: `{word}` after SIM is none of SIM's selectors"
                );
            } else {
                assert!(
                    registry.spec_of(word).is_some(),
                    "{line}: `{word}` is no registered verb{}",
                    nearest(registry, word)
                );
            }
            after_sim = word == "SIM";
            checked += 1;
        }
    }
    // SIM and TEAM twice, @tutorial, BOARD, STARTUP, VIEW, LOCK and FIRE.
    assert_eq!(
        checked, 10,
        "the hints name the verbs they were written to name"
    );
}
