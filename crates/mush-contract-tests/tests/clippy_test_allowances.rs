//! **The clippy test allowances** — a workspace policy gate (WS01-S11A).
//!
//! `.github/workflows/ci.yml` runs `cargo clippy --workspace --all-targets --
//! -D warnings`, so any restriction lint this workspace ever enables is a hard
//! error on every target, test targets included. Clippy does not exempt test
//! code on its own: the four `allow-*-in-tests` keys in the repo-root
//! `clippy.toml` are what keep an `unwrap()` in a `#[cfg(test)] mod tests`
//! block out of a production lint's count.
//!
//! The file enables nothing, which is exactly why it is easy to delete or flip
//! by accident — and doing so quadruples the surface every per-crate enabling
//! decision was sized against (14,137 hits for the six candidate lints without
//! it, 3,725 with it). This test is the tripwire: the config exists, sits at
//! the workspace root where clippy actually reads it, and still says `true`.
//!
//! It is not a claim that the allowance covers all of `tests/`. Clippy exempts
//! `#[cfg(test)]` modules and `#[test]` function bodies only; free helper
//! functions in `crates/*/tests/*.rs` remain in scope, which is why every
//! per-crate enable is measured with `--all-targets`.

use std::fs;
use std::path::{Path, PathBuf};

/// Every allowance the restriction lints in flight depend on.
const REQUIRED_ALLOWANCES: [&str; 4] = [
    "allow-unwrap-in-tests",
    "allow-expect-in-tests",
    "allow-panic-in-tests",
    "allow-indexing-slicing-in-tests",
];

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

#[test]
fn the_repo_root_clippy_config_allows_the_panic_shaped_lints_in_tests() {
    let root = repo_root();
    let path = root.join("clippy.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "clippy reads its configuration from the workspace root and nowhere else; \
             {} is missing ({err}), so every restriction lint this workspace enables \
             counts test code as production",
            path.display()
        )
    });

    let config: toml::Table = toml::from_str(&text)
        .unwrap_or_else(|err| panic!("{} does not parse as TOML: {err}", path.display()));

    for key in REQUIRED_ALLOWANCES {
        let value = config.get(key).unwrap_or_else(|| {
            panic!(
                "{} does not set `{key}` — the lint it exempts would count \
                 `#[cfg(test)]` code, which is not the surface the per-crate \
                 enabling decisions were sized against",
                path.display()
            )
        });
        assert_eq!(
            value.as_bool(),
            Some(true),
            "{} sets `{key} = {value}`; it must be `true`",
            path.display()
        );
    }
}

/// The sibling half: the config must not sit anywhere clippy will not read it.
/// A `crates/<pkg>/clippy.toml` is only honoured when clippy is invoked from
/// that directory, so a per-crate copy is a silent no-op under CI's
/// workspace-rooted run.
#[test]
fn no_crate_carries_its_own_clippy_config() {
    let crates_dir = repo_root().join("crates");
    let entries = fs::read_dir(&crates_dir).expect("the workspace has a crates/ directory");
    let mut strays = Vec::new();
    for entry in entries.flatten() {
        let stray = entry.path().join("clippy.toml");
        if stray.is_file() {
            strays.push(stray.display().to_string());
        }
    }
    strays.sort();
    assert!(
        strays.is_empty(),
        "clippy reads only the config at the directory it runs from; these are dead files \
         under CI's workspace-rooted invocation:\n  {}",
        strays.join("\n  ")
    );
}
