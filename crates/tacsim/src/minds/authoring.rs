//! Validation at the character library's immutable-save boundary.
//!
//! Working text may be invalid. A revision must have a canonical, valid card
//! envelope and compile in the same bounded sandbox used at execution. Admission
//! against a particular core, chassis, Compact floor and operator policy happens
//! at selection and sealing, because those are not properties of the source.

use mush_mind::{Digest, EngineLimits, Sandbox};
use mush_roll::{Envelope, RevisionAdapter, RollError};
use sha2::{Digest as _, Sha256};

use crate::arena::core::CoreClass;

/// Game semantics supplied to the workflow owner. Validation uses a disposable
/// sandbox, so unsuccessful saves cannot grow the process-wide execution cache.
#[derive(Clone, Copy, Debug, Default)]
pub struct RevisionRules;

impl RevisionAdapter for RevisionRules {
    fn normalize_source(&self, source: &str) -> Result<String, RollError> {
        super::normalize::normalize_source(source)
    }

    fn validate(&self, source: &str, envelope: &Envelope) -> Result<(), RollError> {
        validate_source(source, envelope, true)
    }
}

/// Pure re-conformance validation of an immutable retained filed envelope.
/// This is not source/filing authority. The closed Registry reader and typed
/// producer independently prove the retained filed subject. Author saves keep
/// their exact current-ABI requirement; source and old metadata are unchanged.
pub(crate) fn validate_retained_registry(
    source: &str,
    envelope: &Envelope,
) -> Result<(), RollError> {
    validate_source(source, envelope, false)
}
fn validate_source(source: &str, envelope: &Envelope, current_save: bool) -> Result<(), RollError> {
    envelope.validate()?;
    if source.len() > mush_roll::MAX_SOURCE_BYTES {
        return Err(invalid("source", "maximum 32 KiB"));
    }
    if current_save && envelope.mind_abi != super::MIND_ABI {
        return Err(invalid("mind_abi", "not the current executable ABI"));
    }
    let class = CoreClass::parse(&envelope.declared_class)
        .ok_or_else(|| invalid("declared_class", "expected I, II or III"))?;
    if class == CoreClass::Zero {
        return Err(invalid(
            "declared_class",
            "class 0 is the Registry drive ROM; authored scripts require I, II or III",
        ));
    }
    crate::arena::schema::validate_envelope(
        &envelope.schema,
        &envelope.presets,
        &envelope.profiles,
        &envelope.rules,
        class,
    )
    .map_err(|error| invalid("envelope", error.to_string()))?;
    super::metadata::validate(source, envelope)?;
    let sandbox = Sandbox::new(&EngineLimits::default(), super::register_api);
    let digest = Digest(Sha256::digest(source.as_bytes()).into());
    sandbox
        .compile(source, digest, super::MIND_ABI)
        .map_err(|error| invalid("source", error.to_string()))?;
    Ok(())
}

fn invalid(field: &str, message: impl Into<String>) -> RollError {
    RollError::Invalid {
        field: field.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope() -> Envelope {
        Envelope {
            schema: json!({"sections": []}),
            presets: json!({}),
            profiles: json!({"Default": {}}),
            rules: json!({}),
            declared_class: "I".to_owned(),
            mind_abi: crate::minds::MIND_ABI.to_owned(),
        }
    }

    #[test]
    fn saving_requires_the_real_think_signature_and_sandbox() {
        let rules = RevisionRules;
        let envelope = envelope();
        assert!(rules.validate("fn think(view, card) {}", &envelope).is_ok());
        for source in [
            "fn think(view) {}",
            "fn think(view, card) { missing_name; }",
            "fn think(view, card) { eval(\"42\"); }",
            "fn think(view, card) { import \"outside\"; }",
        ] {
            assert!(rules.validate(source, &envelope).is_err(), "{source}");
        }
    }

    #[test]
    fn save_does_not_execute_top_level_statements() {
        assert!(
            RevisionRules
                .validate("while true {} fn think(view, card) {}", &envelope())
                .is_ok()
        );
    }

    #[test]
    fn saving_refuses_unknown_abi_rom_claim_and_invalid_profile_values() {
        let source = "fn think(view, card) {}";
        let mut metadata = envelope();
        metadata.mind_abi = "old".to_owned();
        assert!(RevisionRules.validate(source, &metadata).is_err());
        metadata = envelope();
        metadata.declared_class = "0".to_owned();
        assert!(RevisionRules.validate(source, &metadata).is_err());
        metadata = envelope();
        metadata.profiles = json!({"Default": {"undeclared": 1}});
        assert!(RevisionRules.validate(source, &metadata).is_err());
    }
}
