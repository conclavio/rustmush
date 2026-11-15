#![cfg(test)]
//! Doc 46 CT.9: the Registry twin's identity and its seal.
use super::{ExactRecall, Review, TermRevocation};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Every identity constant of an issuer, as it reads.
fn identity<K: Review>() -> Vec<String> {
    vec![
        K::AUTHORITY.to_owned(),
        K::ACTION.to_owned(),
        K::OUTCOME.to_owned(),
        K::CAPABILITY.to_owned(),
        K::UNAVAILABLE.to_owned(),
        K::CAPTURED.to_owned(),
        K::OWNERS.join(","),
        serde_json::to_string(&K::action_value()).unwrap_or_default(),
    ]
}

/// The recall's identity strings hold the bytes its files held before CT.9.
#[test]
fn recall_identity_strings_hold_todays_bytes() -> TestResult {
    check_eq!(
        identity::<ExactRecall>(),
        [
            "mind_recall",
            "registry_revision_recall",
            "registry_revision_recalled",
            "roll.registry.recall",
            "recall definitions unavailable",
            "recall readiness already captured",
            "roll,org",
            r#"{"format":1,"protocol":"registry_exact_revision_recall_v1","reason":"registry_review","scope":"one_filed_revision","shared_rating":"unchanged"}"#,
        ]
    );
    Ok(())
}

/// The revocation's identity strings hold the bytes its files held before CT.9.
#[test]
fn revocation_identity_strings_hold_todays_bytes() -> TestResult {
    check_eq!(
        identity::<TermRevocation>(),
        [
            "mind_revocation",
            "registry_term_revocation",
            "registry_term_revoked",
            "roll.registry.revoke",
            "revocation definitions unavailable",
            "revocation readiness already captured",
            "roll,org,economy",
            r#"{"duration":"original_grace_end","format":1,"later_terms":"fresh_admission","protocol":"registry_exact_term_revocation_v1","reason":"registry_review","scope":"one_accepted_term_generation","shared_rating":"unchanged"}"#,
        ]
    );
    Ok(())
}

/// The census for a later edit that widens the seal instead of going round it: exactly two
/// `impl Sealed for` the twin's issuers, one in each family's `policy.rs`; exactly two
/// `impl Review for`, both in `registry_twin.rs`; one `mod seal {`; and no `dyn` beside the
/// trait's name. A census of the source, not a proof: the compiler is the proof.
#[test]
fn the_review_seal_admits_exactly_two() -> TestResult {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> TestResult {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                sources(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files)?;
    files.sort();
    let dynamic = [["dyn ", "Review"].concat(), ["dyn ", "Sealed"].concat()];
    let (mut sealed, mut reviews, mut seals) = (Vec::new(), Vec::new(), Vec::new());
    for path in &files {
        let name = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        for line in std::fs::read_to_string(path)?.lines() {
            let line = line.trim();
            for issuer in ["ExactRecall", "TermRevocation"] {
                if line == ["impl Sealed for ", issuer, " {"].concat() {
                    sealed.push(name.clone());
                }
            }
            if line.starts_with(["impl Review", " for "].concat().as_str()) {
                reviews.push(name.clone());
            }
            if line == ["pub(crate) mod ", "seal {"].concat() {
                seals.push(name.clone());
            }
            for d in &dynamic {
                check!(!line.contains(d.as_str()), "{name}: {line}");
            }
        }
    }
    check_eq!(
        sealed,
        [
            "commerce/registry_recall/policy.rs",
            "commerce/registry_revocation/policy.rs"
        ]
    );
    check_eq!(
        reviews,
        ["commerce/registry_twin.rs", "commerce/registry_twin.rs"]
    );
    check_eq!(seals, ["commerce/registry_twin.rs"]);
    Ok(())
}
