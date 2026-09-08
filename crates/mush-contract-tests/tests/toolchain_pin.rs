//! **The toolchain pin** — a workspace policy gate (WS01-S11B).
//!
//! The panic-surface gates are measurements: a per-crate source-scan budget,
//! and restriction lints enforced by `.github/workflows/ci.yml`'s
//! `cargo clippy --workspace --all-targets -- -D warnings`. A measurement needs
//! a fixed subject. While CI installed a floating `stable` — seven jobs each
//! naming it themselves — a lint that reports zero in a crate today reported
//! two on the next release, and the gate red-lighted main for every unrelated
//! PR with nothing in the repo to explain it.
//!
//! `rust-toolchain.toml` at the workspace root is now the only place a Rust
//! version is named; CI installs it with an argument-less
//! `rustup toolchain install`, which reads that file. This test is the
//! tripwire on both halves of that arrangement:
//!
//! - the pin is a **specific** version. `channel = "stable"` parses, installs
//!   and passes every other test in this workspace while pinning nothing;
//! - **no workflow names a toolchain of its own**, in any of the forms that
//!   would silently take precedence over the file — a `dtolnay/rust-toolchain@`
//!   ref, a `toolchain:` input, `rustup default`/`rustup override`, or a
//!   `RUSTUP_TOOLCHAIN` variable.
//!
//! The two scans below are separated from the assertions that use them so each
//! can be run against a fixture: a gate that has never been shown to trip is
//! not yet a gate.

use std::fs;
use std::path::{Path, PathBuf};

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// The `[toolchain]` table of a `rust-toolchain.toml`, as rustup reads it.
struct Pin {
    channel: String,
    components: Vec<String>,
}

/// Parse a `rust-toolchain.toml` and insist the channel it names is an exact
/// released version — `1.98.0`, not `stable`, `beta`, `nightly`, a dated
/// nightly, or a partial `1.98` that still floats across point releases.
fn exact_pin(text: &str) -> Result<Pin, String> {
    let config: toml::Table = toml::from_str(text).map_err(|err| format!("not TOML: {err}"))?;
    let table = config
        .get("toolchain")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "no `[toolchain]` table".to_string())?;
    let channel = table
        .get("channel")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "`[toolchain]` sets no string `channel`".to_string())?
        .to_string();

    let parts: Vec<&str> = channel.split('.').collect();
    let exact = parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    if !exact {
        return Err(format!(
            "`channel = \"{channel}\"` is not an exact version; a floating channel gives the \
             panic budget and the clippy gates no fixed compiler to be measured against"
        ));
    }

    let components = table
        .get("components")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(Pin {
        channel,
        components,
    })
}

/// Every way a workflow file can name a toolchain that takes precedence over
/// (or quietly sits beside) `rust-toolchain.toml`, with what it does.
const OVERRIDE_FORMS: [(&str, &str); 5] = [
    (
        "dtolnay/rust-toolchain@",
        "the action's git ref *is* the toolchain, and it refuses to read rust-toolchain.toml",
    ),
    (
        "actions-rs/toolchain",
        "installs whatever its `toolchain` input names",
    ),
    (
        "rustup default",
        "changes the default toolchain out from under the pin",
    ),
    (
        "rustup override",
        "writes a directory override that beats rust-toolchain.toml",
    ),
    (
        "RUSTUP_TOOLCHAIN",
        "the environment variable beats rust-toolchain.toml for every proxy",
    ),
];

/// Every line of `workflow` that names a toolchain of its own, as
/// `line <n>: <text> — <why it wins>`.
fn toolchain_overrides_in(workflow: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (number, line) in workflow.lines().enumerate() {
        let trimmed = line.trim();
        // A YAML `toolchain:` key — the input form of the same override. The
        // prose spelling ("Rust toolchain (pinned by ...)") carries no colon.
        let named_input = trimmed.starts_with("toolchain:") || trimmed.starts_with("- toolchain:");
        for (needle, why) in OVERRIDE_FORMS {
            if trimmed.contains(needle) {
                found.push(format!("line {}: {trimmed} — {why}", number + 1));
            }
        }
        if named_input {
            found.push(format!(
                "line {}: {trimmed} — a `toolchain:` input names a version the file does not",
                number + 1
            ));
        }
    }
    found
}

/// Every workflow file under `.github/workflows`, sorted.
fn workflow_files(root: &Path) -> Vec<PathBuf> {
    let dir = root.join(".github").join("workflows");
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|err| panic!("{} is not readable: {err}", dir.display()));
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "{} holds no workflow files; the scan below would pass vacuously",
        dir.display()
    );
    files
}

#[test]
fn the_repo_pins_an_exact_rust_toolchain_with_the_components_ci_gates_need() {
    let path = repo_root().join("rust-toolchain.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "{} is missing ({err}); without it rustup resolves whatever `stable` is on the day, \
             and every panic-budget and clippy measurement in this workspace is against an \
             unknown compiler",
            path.display()
        )
    });

    let pin = exact_pin(&text)
        .unwrap_or_else(|why| panic!("{} does not pin a compiler: {why}", path.display()));

    // CI installs the toolchain from this file alone, so the components the
    // gates run are the ones it lists. `cargo fmt --all --check` and the clippy
    // gate both live in the `test` job.
    for component in ["clippy", "rustfmt"] {
        assert!(
            pin.components.iter().any(|have| have == component),
            "{} pins {} but does not list `{component}` in `components`; CI installs the \
             toolchain from this file, so the gate that needs it fails on a fresh runner",
            path.display(),
            pin.channel
        );
    }
}

/// rustup also honours an extension-less `rust-toolchain` file. Two files is
/// one file too many: the loser is edited, believed, and enforces nothing.
#[test]
fn no_second_toolchain_file_sits_beside_the_pin() {
    let legacy = repo_root().join("rust-toolchain");
    assert!(
        !legacy.is_file(),
        "{} exists beside rust-toolchain.toml; rustup reads one of them and the other is a \
         version nobody is running",
        legacy.display()
    );
}

#[test]
fn no_workflow_names_a_rust_version_of_its_own() {
    let root = repo_root();
    for path in workflow_files(&root) {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{} is not readable: {err}", path.display()));
        let overrides = toolchain_overrides_in(&text);
        assert!(
            overrides.is_empty(),
            "{} names a toolchain instead of installing the one rust-toolchain.toml pins:\n  {}",
            path.display(),
            overrides.join("\n  ")
        );
    }
}

#[test]
fn the_pin_check_rejects_every_channel_that_still_floats() {
    for channel in [
        "stable",
        "beta",
        "nightly",
        "nightly-2026-01-01",
        // Partial versions resolve to the newest matching point release, so
        // this one moves under the gates the same way `stable` does.
        "1.98",
    ] {
        let text = format!("[toolchain]\nchannel = \"{channel}\"\n");
        assert!(
            exact_pin(&text).is_err(),
            "`channel = \"{channel}\"` was accepted as a pin"
        );
    }

    let pinned = exact_pin("[toolchain]\nchannel = \"1.98.0\"\ncomponents = [\"clippy\"]\n")
        .expect("an exact version is a pin");
    assert_eq!(pinned.channel, "1.98.0");
    assert_eq!(pinned.components, vec!["clippy".to_string()]);

    assert!(
        exact_pin("[toolchain]\ncomponents = [\"clippy\"]\n").is_err(),
        "a `[toolchain]` table with no channel was accepted"
    );
}

/// The workflow scan is only a tripwire if it trips. Each form below is one a
/// job could plausibly reacquire, and each would beat the file silently.
#[test]
fn the_workflow_scan_catches_a_job_that_names_its_own_toolchain() {
    let clean = "\
jobs:
  test:
    steps:
      - uses: actions/checkout@v4
      - name: Rust toolchain (pinned by rust-toolchain.toml)
        shell: bash
        run: |
          rustup toolchain install
          rustc --version
";
    assert!(
        toolchain_overrides_in(clean).is_empty(),
        "the shape CI actually uses was reported as an override: {:?}",
        toolchain_overrides_in(clean)
    );

    for (fixture, form) in [
        (
            "      - uses: dtolnay/rust-toolchain@stable\n",
            "action ref",
        ),
        ("        with:\n          toolchain: nightly\n", "input"),
        ("        run: rustup default stable\n", "default"),
        (
            "        run: rustup override set nightly-2026-01-01\n",
            "override",
        ),
        ("      RUSTUP_TOOLCHAIN: nightly\n", "environment"),
    ] {
        let text = format!("{clean}{fixture}");
        assert_eq!(
            toolchain_overrides_in(&text).len(),
            1,
            "the {form} form went unreported: {fixture}"
        );
    }
}
