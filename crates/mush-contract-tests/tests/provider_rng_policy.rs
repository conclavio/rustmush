//! Architectural tripwire for C5: authoritative provider implementation
//! files may obtain randomness only through `PrepareCtx`.

use std::path::{Path, PathBuf};

const PROVIDER_MARKER: &str = "impl ActionProvider for";
const FORBIDDEN_AMBIENT_RNG_MARKERS: &[&str] = &[
    "rand::",
    "rand_chacha",
    "thread_rng",
    "RngCore",
    "SeedableRng",
    "OsRng",
    "OsEntropy",
];

fn rust_sources(root: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot scan {}: {error}", root.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("cannot read entry in {}: {error}", root.display()));
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let kind = entry
            .file_type()
            .unwrap_or_else(|error| panic!("cannot inspect {}: {error}", path.display()));
        if kind.is_dir() {
            if entry.file_name() == "tests" {
                continue;
            }
            rust_sources(&path, files);
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn authoritative_provider_files_have_no_ambient_rng_access() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_sources(&crates, &mut files);

    let mut providers = Vec::new();
    let mut violations = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        if !source.contains(PROVIDER_MARKER) {
            continue;
        }
        providers.push(path.clone());
        for marker in FORBIDDEN_AMBIENT_RNG_MARKERS {
            if source.contains(marker) {
                violations.push(format!("{} contains {marker:?}", path.display()));
            }
        }
    }

    assert!(
        !providers.is_empty(),
        "policy scan found no action providers"
    );
    assert!(
        violations.is_empty(),
        "authoritative providers must use PrepareCtx only:\n{}",
        violations.join("\n")
    );
}
