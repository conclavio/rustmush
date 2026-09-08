//! Why the derived traversals over `LockExpr` need no guard of their own.
//!
//! `LockExpr` derives `Clone`, `PartialEq`, `Debug` and `Serialize`, and every
//! one of them is a recursive walk the compiler wrote. Two of them are on hot
//! paths that must not die: `obj.clone()` in `batch_from_dirty`
//! (`crates/mush-server/src/persist.rs`) runs on the **world task**, and
//! `serde_json::to_string(obj)` (`crates/mush-store/src/redis.rs`) runs on the
//! **store-writer task**. WS01-S2 hand-wrote `Drop` and bounded `eval_lock`
//! and `Display`; it deliberately did *not* hand-write those four, and this
//! file is that decision's argument rather than a lint.
//!
//! The argument is a construction invariant, not a bound: a `Not`/`And`/`Or`
//! node can only enter RAM through one of exactly two doors, and both are
//! bounded already.
//!
//! 1. `parse_lock`, which refuses anything over `MAX_LOCK_EXPR_DEPTH` (48).
//! 2. serde decode, which cannot get past serde_json's own 128-level recursion
//!    limit (WS01-S3 clamps it the rest of the way, to the parser's own 48).
//!
//! Nothing else builds one, so no tree taller than a parser-legal key is ever
//! handed to `Clone`/`Serialize` in the first place. The invariant is worth
//! exactly as much as its enforcement, which is this scan: the day a module or
//! a command handler assembles `LockExpr::And(…)` by hand, the second premise
//! is gone and the derived walks become reachable on an unbounded tree.
//!
//! The scan covers `crates/*/src` only. Test code builds these nodes freely —
//! the tall trees in `lock_depth_eval.rs` and `lock_depth_drop.rs` are built
//! exactly this way, on purpose, on their own threads. It is production code
//! that must stay honest.

use std::fs;
use std::path::{Path, PathBuf};

/// The one file allowed to build compound lock nodes, repo-relative.
const HOME: &str = "crates/mush-core/src/lock.rs";

/// The three constructions the invariant is about.
const COMPOUND: [&str; 3] = ["LockExpr::Not(", "LockExpr::And(", "LockExpr::Or("];

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// Every `.rs` file under `crates/*/src`, as (repo-relative path, contents).
fn crate_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let crates = root.join("crates");
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = fs::read_dir(&crates)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", crates.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("src"))
        .filter(|src| src.is_dir())
        .collect();
    assert!(!stack.is_empty(), "no crates/*/src directories found");
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("{} is unreadable: {err}", dir.display()))
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = code_only(
                    &fs::read_to_string(&path)
                        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display())),
                );
                let name = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, text));
            }
        }
    }
    out.sort();
    out
}

/// One source file with its `#[cfg(test)]` tail and its whole-line comments
/// removed: a policy scan reads production code. Test modules that plant
/// over-deep fixtures build them through `LockExpr::and_chain`, the one
/// builder `lock.rs` lends them, and prose about `LockExpr::And(…)` —
/// including this file's own argument, quoted in `lock.rs`'s docs — is not
/// code either.
fn code_only(text: &str) -> String {
    let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
    text[..boundary]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Which of the three tokens a source file mentions.
fn compound_mentions(text: &str) -> Vec<&'static str> {
    COMPOUND
        .into_iter()
        .filter(|token| text.contains(token))
        .collect()
}

#[test]
fn only_lock_rs_builds_compound_lock_nodes() {
    let offenders: Vec<(String, Vec<&str>)> = crate_sources()
        .into_iter()
        .filter(|(name, _)| name != HOME)
        .map(|(name, text)| {
            let hits = compound_mentions(&text);
            (name, hits)
        })
        .filter(|(_, hits)| !hits.is_empty())
        .collect();

    assert!(
        offenders.is_empty(),
        "these production sources name a compound `LockExpr` variant: {offenders:?}.\n\
         `Clone`, `PartialEq`, `Debug` and `Serialize` on `LockExpr` are derived, hence \
         recursive, and two of them run on tasks that may not die (the world task clones \
         objects in persist.rs, the store writer serializes them in redis.rs). They are \
         safe only because every tree in RAM came through `parse_lock` (bounded at \
         MAX_LOCK_EXPR_DEPTH) or through a decode that serde_json's own limit bounds. A \
         third builder breaks that premise.\n\
         If the hit only *reads* the variant (a `match` arm), this ratchet is stricter \
         than the invariant needs and may be loosened deliberately — but read the derives \
         first, and if you are building, bound what you build (`LockExpr::clamp_depth`)."
    );
}

#[test]
fn the_scan_trips_on_a_file_that_deserves_it() {
    // A gate that has never been shown to trip is not yet a gate. This is the
    // shape it exists to catch — a handler assembling a tree in a loop, which
    // is how an unbounded one would appear.
    let builder = "fn all(of: Vec<LockExpr>) -> LockExpr {\n    \
         let mut e = LockExpr::Const(true);\n    \
         for x in of { e = LockExpr::And(Box::new(e), Box::new(x)); }\n    e\n}\n";
    assert_eq!(
        compound_mentions(&code_only(builder)),
        vec!["LockExpr::And("]
    );

    // A file that only mentions the leaves is clean...
    let leaves = "let e = LockExpr::Is(who);\n";
    assert!(compound_mentions(&code_only(leaves)).is_empty());
    // ...and so is prose, including the sentence above this test.
    let prose = "// we could build a LockExpr::And( here, but we do not\n";
    assert!(compound_mentions(&code_only(prose)).is_empty());
    // But prose does not launder code on the same line.
    let hidden = "let e = LockExpr::Not(Box::new(inner)); // not a comment line\n";
    assert_eq!(
        compound_mentions(&code_only(hidden)),
        vec!["LockExpr::Not("]
    );
}

#[test]
fn the_home_file_is_where_it_is_expected_to_be() {
    // The allowlist is one path; if lock.rs moves, the test above silently
    // stops covering the one file it was written for.
    let sources = crate_sources();
    let home = sources
        .iter()
        .find(|(name, _)| name == HOME)
        .unwrap_or_else(|| panic!("{HOME} is gone; the allowlist needs repointing"));
    assert_eq!(
        compound_mentions(&home.1).len(),
        COMPOUND.len(),
        "{HOME} no longer builds all three compound variants; has the AST moved?"
    );
}
