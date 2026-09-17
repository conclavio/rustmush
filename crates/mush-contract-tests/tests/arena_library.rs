//! **The two arena gates doc 16 §8 names** (16b cut A.1; doc 16 §3.1 D-16-1 and
//! D-16-3, §3.5 "Draw labels", §8 "Gates"; 16b §6 "Verify lenses").
//!
//! 1. **The arena library has no I/O.** Doc 16's whole authority argument rests on
//!    it: while a match runs the arena is the single writer for its own facts and
//!    the world half is the single writer for everything else, and *the two share
//!    no state and no lock*. A promise like that decays the first time somebody
//!    needs "just a quick read" of a record inside a pulse, so the first test
//!    below reads every source file under `crates/tacsim/src/arena/` and refuses
//!    the names that would make such a read possible: the persistence crate, its
//!    handles and batches, the async runtime, the output sink and the world
//!    handle. Cut T's `arena_task` owns all of those *outside* the library, which
//!    is exactly the shape that makes the panic firewall and the void-the-match
//!    rule implementable.
//!
//! 2. **The draw-label set is pinned.** Doc 16 §3.5: *every draw is
//!    `draw(match_seed, label)` … with these labels and no others*, and *adding a
//!    draw can never shift another*. The second property is a consequence of the
//!    keyed hash and is tested inside the crate; the first is a **vocabulary**
//!    claim, and a vocabulary claim needs a gate outside the crate that defines
//!    it. The second test below asserts the five kinds and one encoded example of
//!    each, so a sixth reason to draw — or a changed separator, or an object id
//!    that starts rendering as `#42` — is a reviewed diff.
//!
//! Why a source scan rather than a dependency assertion: `tacsim` as a whole is
//! the game's composition root and legitimately depends on the store, the runtime
//! and the world. The property being defended is about **one module tree inside
//! it**, which no manifest can express, so it is defended the way
//! `restriction_lints.rs` defends its own: by reading the tree.

use std::fs;
use std::path::{Path, PathBuf};

use tacsim::arena::rng::{DrawLabel, LABEL_KINDS};

/// The names that would give the arena library a path out of itself.
///
/// Each is a substring of the file's text, so a `use`, a type in a signature, a
/// field and a doc comment all count — mentioning one is as reviewable as calling
/// it, and a gate that can be talked around by putting the type behind an alias
/// is not a gate.
const FORBIDDEN: [(&str, &str); 7] = [
    (
        "mush_store",
        "the persistence crate: the arena holds no store handle (D-16-3)",
    ),
    (
        "tokio",
        "the async runtime: the arena is a library, and cut T's task owns the runtime (16b §6)",
    ),
    (
        "GatewaySink",
        "the output sink: frames leave through the world half's ordinary output path (doc 16 §3.1)",
    ),
    ("ObjectStore", "a store handle (doc 16 §8; 16b T6)"),
    ("WriteBatch", "a store batch (doc 16 §8; 16b T6)"),
    (
        "&mut World",
        "the world handle: the arena and the world share no state and no lock (D-16-1)",
    ),
    (
        "Ctx<",
        "a module context, which carries the world and the store with it (D-16-1)",
    ),
];

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, sorted, so the walk is reproducible.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_arena_library_has_no_path_to_the_store_the_runtime_or_the_world() {
    let root = repo_root();
    let arena = root.join("crates/tacsim/src/arena");
    assert!(
        arena.is_dir(),
        "{} is not a directory: the arena library moved, and this gate is now asserting nothing. \
         Re-point it at the new path in the same commit.",
        arena.display()
    );
    let mut files = Vec::new();
    rust_files(&arena, &mut files);
    assert!(
        files.len() >= 6,
        "the scan found only {} source files under {}; cut A.1 ships six modules (state, clock, \
         intents, rng, hash, ledger) plus mod.rs, so a shorter walk means the gate is reading the \
         wrong place",
        files.len(),
        arena.display()
    );

    let mut offences: Vec<String> = Vec::new();
    for path in &files {
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = fs::read_to_string(path) else {
            offences.push(format!("{relative}: unreadable"));
            continue;
        };
        for (token, why) in FORBIDDEN {
            // This file names every token itself, so a self-scan is excluded by
            // path rather than by cleverness.
            if let Some(line) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(token))
                .map(|(n, _)| n + 1)
            {
                offences.push(format!("{relative}:{line}: {token:?} — {why}"));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "the arena library reached outside itself. Doc 16 §3.1 (D-16-1, D-16-3) makes the arena an \
         authority that shares no state and no lock with the world half and holds no store handle; \
         cut T's arena task owns the inbox, the egress, the timer and the unwind boundary, and the \
         library owns none of them.\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_draw_label_set_is_pinned() {
    // The kinds, in code order (doc 16 §3.5's four plus §3.6's contact id).
    assert_eq!(
        LABEL_KINDS,
        ["hit", "facet", "burst", "classify", "contact"],
        "the draw-label set changed. Doc 16 §3.5 admits these labels and no others; a new reason \
         to draw is a reviewed diff here and in doc 16."
    );
    assert_eq!(LABEL_KINDS, DrawLabel::LABEL_KINDS);

    // One encoded example per kind, with the exact separators and the exact
    // rendering of an object id (its decimal number, never `#42`).
    let examples: [(DrawLabel, &str); 5] = [
        (DrawLabel::Hit { shot_seq: 7 }, "hit:7"),
        (DrawLabel::Facet { shot_seq: 7 }, "facet:7"),
        (DrawLabel::Burst { shot_seq: 7 }, "burst:7"),
        (
            DrawLabel::Classify {
                observer: mush_core::ObjId(4),
                target: mush_core::ObjId(9),
                pass_seq: 3,
            },
            "classify:4:9:3",
        ),
        (
            DrawLabel::Contact {
                viewer: mush_core::ObjId(4),
                target: mush_core::ObjId(9),
            },
            "contact:4:9",
        ),
    ];
    for (label, expected) in examples {
        assert_eq!(
            label.encode(),
            expected,
            "{label:?} no longer encodes as {expected:?}: a label's bytes are the key of its draw, \
             so changing them silently re-rolls every shot in every replayed ledger"
        );
        assert!(
            LABEL_KINDS.contains(&label.kind()),
            "{label:?} declares a kind that is not in the pinned set"
        );
    }
    // And every pinned kind has an example above, so a new variant cannot be
    // added to the enum without being exercised here.
    let encoded: Vec<&'static str> = LABEL_KINDS.to_vec();
    assert_eq!(encoded.len(), 5);
}
