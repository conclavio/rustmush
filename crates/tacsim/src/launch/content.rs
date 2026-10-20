//! Fixed source import recipes; certificates and filings are produced later.
use mush_core::{DefinitionDescriptor, DefinitionId, DefinitionKind, ModError};
use mush_roll::{WorkingCopy, source_digest};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const FILING_MINOR: i64 = 10_000;
pub const HOUSE_MINOR: i64 = 40_000;
pub const TERM_SECONDS: i64 = 90 * 86_400;
pub const GRACE_SECONDS: i64 = 3 * 86_400;
pub const OFFER_SECONDS: i64 = 3 * 86_400;
pub const LEVY_PERMILLE: u16 = 50;
pub const VENUE_PERMILLE: u16 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchRecipe {
    Meridiani,
    Coprates,
    SafeConduct,
}
impl LaunchRecipe {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "meridiani" => Some(Self::Meridiani),
            "coprates" => Some(Self::Coprates),
            "safe_conduct" => Some(Self::SafeConduct),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Meridiani => "Meridiani Field Series",
            Self::Coprates => "Coprates Drill",
            Self::SafeConduct => "Safe Conduct",
        }
    }
    fn bytes(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Meridiani => (
                include_str!("../../content/minds/house/meridiani_field.rhai"),
                include_str!("../../content/minds/house/meridiani_field.envelope.json"),
                include_str!("../../content/minds/house/meridiani_field.manifest.json"),
            ),
            Self::Coprates => (
                include_str!("../../content/minds/house/coprates_drill.rhai"),
                include_str!("../../content/minds/house/coprates_drill.envelope.json"),
                include_str!("../../content/minds/house/coprates_drill.manifest.json"),
            ),
            Self::SafeConduct => (
                include_str!("../../content/minds/house/safe_conduct.rhai"),
                include_str!("../../content/minds/house/safe_conduct.envelope.json"),
                include_str!("../../content/minds/house/safe_conduct.manifest.json"),
            ),
        }
    }
    pub fn draft(self) -> Result<WorkingCopy, ModError> {
        let (source, envelope, manifest) = self.bytes();
        let manifest: serde_json::Value = serde_json::from_str(manifest).map_err(bad)?;
        if manifest.get("content_version").and_then(|v| v.as_u64()) != Some(1)
            || manifest.get("source_sha256").and_then(|v| v.as_str())
                != Some(source_digest(source).to_hex().as_str())
            || manifest.get("source_bytes").and_then(|v| v.as_u64()) != Some(source.len() as u64)
            || manifest.get("envelope_sha256").and_then(|v| v.as_str())
                != Some(source_digest(envelope).to_hex().as_str())
        {
            return Err(bad("shipped launch content digest differs"));
        }
        let draft = WorkingCopy {
            source: source.into(),
            envelope: serde_json::from_str(envelope).map_err(bad)?,
        };
        draft.validate_bounds().map_err(bad)?;
        Ok(draft)
    }
}
pub(super) fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    let recipes = [
        LaunchRecipe::Meridiani,
        LaunchRecipe::Coprates,
        LaunchRecipe::SafeConduct,
    ]
    .into_iter()
    .map(|recipe| {
        let draft = recipe.draft()?;
        Ok(json!({"recipe":recipe,"source":source_digest(&draft.source),"envelope":draft.envelope}))
    })
    .collect::<Result<Vec<_>, ModError>>()?;
    DefinitionDescriptor::new(
        DefinitionKind::new("launch_content").map_err(bad)?,
        DefinitionId::new("named_mind_launch").map_err(bad)?,
        1,
        json!({"format":1,"recipes":recipes,"instrument":"tacsim.credit","minor_scale":2,
            "filing_minor":FILING_MINOR,"house_minor":HOUSE_MINOR,"renewal_minor":HOUSE_MINOR,
            "seats":1,"form":"sealed","revision":"pin","term_seconds":TERM_SECONDS,
            "grace_seconds":GRACE_SECONDS,"offer_seconds":OFFER_SECONDS,
            "registry_permille":LEVY_PERMILLE,"venue_permille":VENUE_PERMILLE}),
    )
    .map_err(bad)
}
pub(super) fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
