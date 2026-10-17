//! Compact editable Standard starter; the full shipped source is reference only.

use crate::arena::card::line;
use crate::arena::core::CoreClass;
use crate::arena::runtime_card::stock_definition;
use crate::config::ReflexConfig;
use mush_roll::{Envelope, RollError, WorkingCopy};

/// The stock helper runs under this ordinary think's operation allowance.
/// Its implementation charges executed Rhai operations plus declared helper fees.
/// No privileged native fallback is called by this source.
pub const STANDARD_STARTER: &str = r#"// Standard starter. Card fields are sealed values; memory is your persistent state.
// Edit the card or add rules without copying the full reference implementation.
// stock::think is metered inside this mind's declared Class I budget.
fn think(view, card) {
    this = stock::think(view, card, this);
}
"#;

/// New local working copy. Defaults are captured now and remain data afterward.
/// # Errors
/// A malformed stock schema/default or metadata bound.
pub fn standard_draft() -> Result<WorkingCopy, RollError> {
    standard_draft_with(&ReflexConfig::default())
}

/// Explicit frozen doctrine defaults for the author command's current environment.
/// # Errors
/// Invalid configured card/schema values, never clamped.
pub fn standard_draft_with(cfg: &ReflexConfig) -> Result<WorkingCopy, RollError> {
    let definition =
        stock_definition(&[("Line".into(), line(cfg))], CoreClass::I).map_err(|e| {
            RollError::Invalid {
                field: e.field,
                message: e.message,
            }
        })?;
    let profile = definition
        .profiles
        .first()
        .ok_or_else(|| RollError::Invalid {
            field: "profiles".into(),
            message: "missing stock opening profile".into(),
        })?;
    let draft = WorkingCopy {
        source: STANDARD_STARTER.into(),
        envelope: Envelope {
            schema: serde_json::to_value(&definition.schema).map_err(|_| RollError::Invalid {
                field: "schema".into(),
                message: "cannot encode stock schema".into(),
            })?,
            presets: serde_json::json!({"Line": profile.values}),
            profiles: serde_json::json!({"Line": profile.values}),
            rules: serde_json::json!({}),
            declared_class: "I".into(),
            mind_abi: super::MIND_ABI.into(),
        },
    };
    draft.validate_bounds()?;
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn m6_standard_starter_is_small_complete_and_copies_config_exactly() {
        let cfg = ReflexConfig {
            weight_range: u32::MAX,
            rally_ms: 20_013,
            ..ReflexConfig::default()
        };
        let draft = standard_draft_with(&cfg).unwrap();
        assert!(draft.source.len() < 1024);
        let fields = draft.envelope.profiles["Line"].as_object().unwrap();
        assert_eq!(fields.len(), 24);
        assert_eq!(fields["weight_range"], u32::MAX);
        assert_eq!(fields["rally_ms"], 20_013);
        assert_eq!(draft.envelope.profiles, draft.envelope.presets);
        crate::arena::schema::validate_envelope(
            &draft.envelope.schema,
            &draft.envelope.presets,
            &draft.envelope.profiles,
            &draft.envelope.rules,
            CoreClass::I,
        )
        .unwrap();
    }
}
