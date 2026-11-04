//! Architectural tripwire for C5: authoritative provider implementation
//! files may obtain randomness only through `PrepareCtx`.

use mush_contract_tests::scan;

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

#[test]
fn authoritative_provider_files_have_no_ambient_rng_access() {
    let crates = scan::repo_root().join("crates");
    let files = scan::files(&crates, &["rs"], &["tests"]);

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
