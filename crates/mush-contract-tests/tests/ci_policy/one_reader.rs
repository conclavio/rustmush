//! **One reader** — a converted crate's stories keep no reader of their own
//! (doc 51 §3.9, unit FL.7).
//!
//! Before doc 51 the gateway's stories carried ten copies of one `Term`
//! struct, and mushd's a dozen `ask_whole`s: a fix to one copy never reached
//! the others, which is how one wait was mended in one story and missed in
//! its twin. The story kit (`mush-storykit`) holds the one `Term`, the one
//! `ask_whole` and the one `strip_ansi`. Once a crate's unit has moved its
//! stories onto them, this gate refuses a new definition of any of the three
//! anywhere under that crate's `tests/`.
//!
//! It is a list of the crates that have converted, so it tightens a unit at a
//! time: doc 51 unit FL.7 put mush-gateway on it, and FL.8 adds mushd. A
//! crate joins in the pull request that removes its last copy.
//!
//! What it reads: every `.rs` file under `crates/<crate>/tests/`, with
//! comments and string literals blanked, for `struct Term`, `fn ask_whole`
//! and `fn strip_ansi` as whole words. A use of the kit's (`Term::play`,
//! `ask_whole(&mut term, …)`) is not a definition and is not counted.

use std::path::Path;

use mush_contract_tests::scan;

/// The crates whose stories have moved onto the kit's reader.
const CONVERTED: [&str; 2] = ["mush-gateway", "mushd"];

/// The kit, which holds the one definition of each.
const KIT_CRATE: &str = "mush-storykit";

/// What the gate refuses: (the item's keyword, its name).
const READERS: [(&str, &str); 3] = [
    ("struct", "Term"),
    ("fn", "ask_whole"),
    ("fn", "strip_ansi"),
];

/// Whether `ch` can be part of an identifier.
fn ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The 1-based lines of `code` (comments and literals already blanked) that
/// define `keyword name`: the keyword and the name as whole words, with only
/// spaces between them.
fn definitions(code: &str, keyword: &str, name: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    for (index, line) in code.lines().enumerate() {
        let mut rest = line;
        let mut offset = 0;
        while let Some(at) = rest.find(keyword) {
            let start = offset + at;
            let before = line[..start].chars().next_back();
            let after_keyword = &line[start + keyword.len()..];
            let spaced = after_keyword.trim_start_matches(' ');
            let is_def = before.is_none_or(|c| !ident_char(c))
                && spaced.len() < after_keyword.len()
                && spaced.starts_with(name)
                && spaced[name.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !ident_char(c));
            if is_def {
                lines.push(index + 1);
                break;
            }
            offset = start + keyword.len();
            rest = &line[offset..];
        }
    }
    lines
}

/// Every definition of a reader under `crate_dir/tests/`, as
/// `<path>:<line>: <keyword> <name>`.
fn readers_under(root: &Path, crate_dir: &Path) -> Vec<String> {
    let tests = crate_dir.join("tests");
    let mut found = Vec::new();
    for path in scan::rust_files(&tests) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let code = scan::strip_comments_and_literals(&source);
        for (keyword, name) in READERS {
            for line in definitions(&code, keyword, name) {
                found.push(format!(
                    "{}:{line}: {keyword} {name}",
                    scan::relative(root, &path)
                ));
            }
        }
    }
    found
}

#[test]
fn a_converted_crate_defines_no_reader_of_its_own() {
    let root = scan::repo_root();
    let mut offenders = Vec::new();
    for krate in CONVERTED {
        let dir = root.join("crates").join(krate);
        assert!(
            dir.join("Cargo.toml").is_file(),
            "one_reader lists `{krate}`, which is not a crate under crates/"
        );
        offenders.extend(readers_under(&root, &dir));
    }
    assert!(
        offenders.is_empty(),
        "a story crate that has moved onto the story kit's reader defines its own again \
         (doc 51 §3.9, `one_reader`):\n  {}\n\nUse `mush_storykit::Term`, \
         `mush_storykit::ask_whole` and `mush_storykit::strip_ansi` (re-exported by the \
         crate's rig). A helper only one suite needs is a free function over `&mut Term`, \
         not a second `Term`. A fix the kit's reader lacks goes into the kit.",
        offenders.join("\n  ")
    );
}

#[test]
fn the_kit_holds_the_one_definition_of_each() {
    let root = scan::repo_root();
    let src = root.join("crates").join(KIT_CRATE).join("src");
    for (keyword, name) in READERS {
        let count: usize = scan::rust_files(&src)
            .iter()
            .map(|path| {
                let source = std::fs::read_to_string(path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                definitions(&scan::strip_comments_and_literals(&source), keyword, name).len()
            })
            .sum();
        assert_eq!(
            count, 1,
            "the story kit must define `{keyword} {name}` exactly once; it defines it {count} \
             times"
        );
    }
}

#[test]
fn the_reader_finds_a_definition_and_not_a_use() {
    let code = "pub struct Term {\n\
                struct Terminal;\n\
                let t = Term::play(addr);\n\
                async fn ask_whole(t: &mut Term) {}\n\
                ask_whole(&mut t, \"x\");\n\
                pub(super) fn strip_ansi(s: &str) {}\n\
                fn strip_ansi_twice() {}\n\
                fn  strip_ansi() {}\n";
    assert_eq!(definitions(code, "struct", "Term"), [1]);
    assert_eq!(definitions(code, "fn", "ask_whole"), [4]);
    assert_eq!(definitions(code, "fn", "strip_ansi"), [6, 8]);
}
