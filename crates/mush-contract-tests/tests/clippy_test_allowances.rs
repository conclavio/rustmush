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
//! by accident — and doing so multiplies the surface every per-crate enabling
//! decision was sized against by roughly four. This test is the tripwire: the
//! config exists at the workspace root, still says `true`, and no crate has
//! planted one of its own where clippy would reach it first.
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
            "every crate here walks up to the workspace root for its clippy config; \
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

/// Both names clippy will accept for a config file, dotted one first — the same
/// order clippy checks them in within a directory.
const CONFIG_FILENAMES: [&str; 2] = [".clippy.toml", "clippy.toml"];

/// Every clippy config sitting one level under `dir`, sorted. Split out from the
/// test below so the scan itself can be exercised against a fixture directory.
fn shadowing_configs_under(dir: &Path) -> Vec<String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => panic!("{} is not readable: {err}", dir.display()),
    };
    let mut strays = Vec::new();
    for entry in entries.flatten() {
        for name in CONFIG_FILENAMES {
            let stray = entry.path().join(name);
            if stray.is_file() {
                strays.push(stray.display().to_string());
            }
        }
    }
    strays.sort();
    strays
}

/// The sibling half: no crate may shadow the root config with its own.
///
/// Clippy's lookup starts at the crate's `CARGO_MANIFEST_DIR` — not at the
/// directory clippy was invoked from — and walks *up* until it finds a config,
/// taking the first hit and reading no other. A `crates/<pkg>/clippy.toml` is
/// therefore found before the workspace root's even under CI's workspace-rooted
/// run, and it silently drops all four allowances for that crate: exactly the
/// failure this file exists to prevent, and quiet, because the shadowing file
/// need not mention the allowances at all.
///
/// Both spellings count. Clippy accepts `.clippy.toml` and `clippy.toml`, and
/// looks for the dotted name first in each directory, so scanning only the
/// undotted one would leave the likelier-to-be-overlooked half uncovered.
#[test]
fn no_crate_carries_its_own_clippy_config() {
    let strays = shadowing_configs_under(&repo_root().join("crates"));
    assert!(
        strays.is_empty(),
        "clippy's config lookup starts at the crate's manifest directory and walks up, taking \
         the first file it finds; these shadow the workspace root's config and silently drop \
         the test allowances for their crate:\n  {}",
        strays.join("\n  ")
    );
}

/// The scan above is only a tripwire if it trips. Both spellings are planted in
/// a fixture tree and both must be reported — the dotted one especially, since
/// clippy reaches it first and a scan for `clippy.toml` alone would wave it
/// through.
#[test]
fn the_stray_scan_catches_both_spellings() {
    let fixture = std::env::temp_dir().join(format!(
        "rustmush-ws01-s11a-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&fixture);
    for (crate_dir, name) in [("dotted-pkg", ".clippy.toml"), ("plain-pkg", "clippy.toml")] {
        let dir = fixture.join(crate_dir);
        fs::create_dir_all(&dir).expect("the fixture tree is creatable under the temp dir");
        fs::write(dir.join(name), "allow-unwrap-in-tests = true\n")
            .expect("the fixture config is writable");
    }
    fs::create_dir_all(fixture.join("clean-pkg")).expect("the clean crate dir is creatable");

    let found = shadowing_configs_under(&fixture);
    let _ = fs::remove_dir_all(&fixture);

    assert_eq!(
        found.len(),
        2,
        "both spellings must be reported, got: {found:?}"
    );
    assert!(
        found.iter().any(|p| p.ends_with(".clippy.toml")),
        "the dotted spelling clippy reads first went unreported: {found:?}"
    );
    let undotted = format!("plain-pkg{}clippy.toml", std::path::MAIN_SEPARATOR);
    assert!(
        found.iter().any(|p| p.ends_with(&undotted)),
        "the undotted spelling went unreported: {found:?}"
    );
}
