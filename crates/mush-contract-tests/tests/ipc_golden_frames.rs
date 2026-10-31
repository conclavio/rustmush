//! **The IPC golden-set lint** — a workspace policy gate (doc 33 §4.1, cut GG.0).
//!
//! The gateway and the server may run one boundary protocol version apart, and
//! the proof that they can is a recorded set of frames per version under
//! `crates/mush-ipc/tests/golden/v<N>/`, written by the build whose
//! `IPC_VERSION` was N (`cargo test -p mush-ipc --test golden
//! record_golden_frames -- --ignored`). `mush-ipc`'s own golden suite checks
//! that the current set matches the codec and that the previous set still
//! decodes — but both of those can only run against a set that EXISTS, and a
//! bump that forgot to record one would pass them vacuously until the next
//! bump found no previous set to decode. This gate closes that hole: the
//! newest recorded set must be exactly `IPC_VERSION`, complete in both
//! directions, and the previous version's set must still be there, because the
//! window reaches back to it.
//!
//! The scan is separated from the verdict so each can be shown to trip on a
//! fixture: a gate that has never failed is not yet a gate.

use std::fs;
use std::path::{Path, PathBuf};

use mush_ipc::IPC_VERSION;

/// The first version with a recorded set; nothing before it is expected.
const FIRST_GOLDEN_VERSION: u32 = 12;

/// The command that records the current version's set.
const RECORD_COMMAND: &str =
    "cargo test -p mush-ipc --test golden record_golden_frames -- --ignored";

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// `crates/mush-ipc/tests/golden/`.
fn golden_root() -> PathBuf {
    repo_root()
        .join("crates")
        .join("mush-ipc")
        .join("tests")
        .join("golden")
}

/// Every version with a `v<N>` directory under `root`, ascending.
fn recorded_versions(root: &Path) -> Vec<u32> {
    let mut versions = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return versions;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(version) = name
            .to_str()
            .and_then(|name| name.strip_prefix('v'))
            .and_then(|digits| digits.parse::<u32>().ok())
        else {
            continue;
        };
        versions.push(version);
    }
    versions.sort_unstable();
    versions
}

/// The number of `.frame` files in one direction of one set.
fn frames_in(root: &Path, version: u32, direction: &str) -> usize {
    fs::read_dir(root.join(format!("v{version}")).join(direction))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|ext| ext.to_str()) == Some("frame")
                })
                .count()
        })
        .unwrap_or(0)
}

/// A set is complete when both directions hold at least one frame.
fn set_is_complete(root: &Path, version: u32) -> Result<(), String> {
    for direction in ["to_server", "to_gateway"] {
        if frames_in(root, version, direction) == 0 {
            return Err(format!(
                "v{version}/{direction}/ holds no .frame file; the set is incomplete"
            ));
        }
    }
    Ok(())
}

/// The verdict for a protocol at `version` over the sets recorded under `root`.
fn check(root: &Path, version: u32) -> Result<(), String> {
    let versions = recorded_versions(root);
    let Some(&newest) = versions.last() else {
        return Err(format!(
            "no recorded golden set under {}; record one with `{RECORD_COMMAND}`",
            root.display()
        ));
    };
    if newest < version {
        return Err(format!(
            "IPC_VERSION is {version} but the newest recorded golden set is v{newest}: a bump \
             records its contract. Run `{RECORD_COMMAND}` at version {version} and commit \
             crates/mush-ipc/tests/golden/v{version}/"
        ));
    }
    if newest > version {
        return Err(format!(
            "a golden set v{newest} is newer than IPC_VERSION {version}: a set is written only by \
             the build at its version, so either a bump was reverted without its set or a \
             directory was renamed; remove or restore so the newest set is v{version}"
        ));
    }
    set_is_complete(root, version)?;
    if let Some(previous) = version
        .checked_sub(1)
        .filter(|v| *v >= FIRST_GOLDEN_VERSION)
    {
        set_is_complete(root, previous).map_err(|e| {
            format!(
                "the previous version's set, which the one-step window decodes under this \
                 build, is missing or empty: {e}"
            )
        })?;
    }
    Ok(())
}

/// The gate itself.
#[test]
fn the_protocol_version_has_a_complete_recorded_golden_set() {
    let root = golden_root();
    check(&root, IPC_VERSION).unwrap_or_else(|e| panic!("{e}"));
    let versions = recorded_versions(&root);
    assert!(
        versions.contains(&FIRST_GOLDEN_VERSION),
        "the first recorded set (v{FIRST_GOLDEN_VERSION}) is history and stays: {versions:?}"
    );
}

// ---------------------------------------------------------------------------
// The scan and the verdict against fixtures: each failure mode shown to trip.
// ---------------------------------------------------------------------------

/// A scratch golden root with the given `(version, complete)` sets.
struct Fixture(PathBuf);

impl Fixture {
    fn new(tag: &str, sets: &[(u32, bool)]) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "rustmush-ipc-golden-lint-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        for &(version, complete) in sets {
            for direction in ["to_server", "to_gateway"] {
                let dir = root.join(format!("v{version}")).join(direction);
                fs::create_dir_all(&dir).expect("fixture dir");
                if complete {
                    fs::write(dir.join("Sample-00.frame"), [0, 0, 0, 1, 0xc0]).expect("frame");
                }
            }
        }
        Fixture(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_bump_with_no_recorded_set_is_refused() {
    let fixture = Fixture::new("missing-13", &[(12, true)]);
    let err = check(&fixture.0, 13).expect_err("13 has no set");
    assert!(
        err.contains("newest recorded golden set is v12") && err.contains(RECORD_COMMAND),
        "{err}"
    );
}

#[test]
fn a_recorded_bump_with_its_predecessor_passes() {
    let fixture = Fixture::new("recorded-13", &[(12, true), (13, true)]);
    check(&fixture.0, 13).expect("12 and 13 recorded");
    assert_eq!(recorded_versions(&fixture.0), vec![12, 13]);
}

#[test]
fn an_empty_set_does_not_count_as_recorded() {
    let fixture = Fixture::new("empty-13", &[(12, true), (13, false)]);
    let err = check(&fixture.0, 13).expect_err("13's directories are empty");
    assert!(err.contains("v13/to_server/ holds no .frame file"), "{err}");
}

#[test]
fn a_missing_previous_set_is_refused() {
    let fixture = Fixture::new("no-12", &[(13, true)]);
    let err = check(&fixture.0, 13).expect_err("the window needs 12");
    assert!(err.contains("previous version's set"), "{err}");
}

#[test]
fn a_set_newer_than_the_protocol_is_refused() {
    let fixture = Fixture::new("future-14", &[(12, true), (13, true), (14, true)]);
    let err = check(&fixture.0, 13).expect_err("14 is newer than the protocol");
    assert!(err.contains("v14 is newer than IPC_VERSION 13"), "{err}");
}

#[test]
fn the_first_version_needs_no_predecessor() {
    let fixture = Fixture::new("first", &[(FIRST_GOLDEN_VERSION, true)]);
    check(&fixture.0, FIRST_GOLDEN_VERSION).expect("nothing is recorded before the first set");
}

#[test]
fn the_scan_ignores_what_is_not_a_set() {
    let fixture = Fixture::new("noise", &[(12, true)]);
    fs::create_dir_all(fixture.0.join("vnext")).expect("noise dir");
    fs::write(fixture.0.join("README.md"), "notes").expect("noise file");
    assert_eq!(recorded_versions(&fixture.0), vec![12]);
}
