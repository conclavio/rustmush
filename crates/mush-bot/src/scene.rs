//! Local evidence artifacts. Neither artifact is a restorable game checkpoint.
use crate::{
    Result,
    conversation::Scope,
    experiment::{load_json, revision},
    memory::{Event, Record, Source},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

const MAX_BOOKMARK: usize = 32_000_000;
const MODULES: &[&str] = &[
    "runner",
    "character",
    "memory",
    "attention",
    "conversation",
    "configuration",
    "accounting",
];
const OPTIONAL_MODULES: &[&str] = &["social", "learning", "activity", "mission", "playtest"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Watermark {
    pub session: String,
    pub scope: Scope,
    pub elapsed_ms: u64,
    pub output_sequence: u64,
    pub observation: u64,
    pub speech: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub version: u32,
    pub required: bool,
    pub sha256: String,
    pub data: Value,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub version: u32,
    pub id: String,
    pub kind: String,
    pub watermark: Watermark,
    pub world_restorable: bool,
    pub boundary: String,
    pub modules: BTreeMap<String, Contribution>,
}
impl Bookmark {
    pub fn new(watermark: Watermark, modules: BTreeMap<String, Value>) -> Result<Self> {
        let modules = modules
            .into_iter()
            .map(|(name, data)| {
                let sha256 = revision(&serde_json::to_vec(&data)?);
                Ok((
                    name,
                    Contribution {
                        version: 1,
                        required: true,
                        sha256,
                        data,
                    },
                ))
            })
            .collect::<Result<_>>()?;
        let mut value = Self {
            version: 1,
            id: String::new(),
            kind: "bot_bookmark".into(),
            watermark,
            world_restorable: false,
            boundary: "locally_paused_and_settled; live world and remote queues are not frozen"
                .into(),
            modules,
        };
        value.id = value.identity()?;
        value.validate()?;
        Ok(value)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = load_json(path, MAX_BOOKMARK)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.watermark.scope.validate()?;
        if self.version != 1
            || self.kind != "bot_bookmark"
            || self.world_restorable
            || self.watermark.session.is_empty()
            || self.watermark.session.len() > 200
            || self.boundary
                != "locally_paused_and_settled; live world and remote queues are not frozen"
            || self.modules.len() > 32
            || serde_json::to_vec(self)?.len() > MAX_BOOKMARK
        {
            return Err(
                "invalid bot bookmark version, boundary or size; world restoration is unsupported"
                    .into(),
            );
        }
        for name in MODULES {
            if !self.modules.get(*name).is_some_and(|m| m.required) {
                return Err(format!("bookmark lacks required {name} contribution").into());
            }
        }
        for (name, module) in &self.modules {
            if !crate::organization::identifier(name)
                || (module.required
                    && ((!MODULES.contains(&name.as_str())
                        && !OPTIONAL_MODULES.contains(&name.as_str()))
                        || module.version != 1))
            {
                return Err(format!(
                    "unsupported required bookmark contribution {name} v{}",
                    module.version
                )
                .into());
            }
            if module.sha256 != revision(&serde_json::to_vec(&module.data)?) {
                return Err(format!("bookmark contribution {name} checksum mismatch").into());
            }
        }
        if self.id != self.identity()? {
            return Err("bookmark identity checksum mismatch".into());
        }
        Ok(())
    }

    fn identity(&self) -> Result<String> {
        Ok(revision(&serde_json::to_vec(
            &serde_json::json!({"version":self.version,
                "kind":self.kind,"watermark":self.watermark,"boundary":self.boundary,"world_restorable":self.world_restorable,
                "contributions":self.modules.iter().map(|(name,m)| (name,m.version,m.required,&m.sha256)).collect::<Vec<_>>()
            }),
        )?))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub episode: u64,
    pub journal_session: u64,
    pub observed_at_ms: u64,
    pub quote: String,
    pub excerpted: bool,
    pub place: Option<crate::memory::Place>,
    pub speaker_labels: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recap {
    pub version: u32,
    pub kind: String,
    pub watermark: Watermark,
    pub scope: String,
    pub incomplete: bool,
    pub coverage: String,
    pub observed_episodes: usize,
    pub omitted_episodes: usize,
    pub provider_requests: u64,
    pub entries: Vec<Citation>,
}
impl Recap {
    pub fn from_observations(watermark: Watermark, session: u64, records: &[Record]) -> Self {
        let observed: Vec<_> = records
            .iter()
            .filter_map(|r| match &r.event {
                Event::Episode(e) if r.session == session && e.source == Source::Observed => {
                    Some((r, e))
                }
                _ => None,
            })
            .collect();
        let entries: Vec<_> = observed
            .iter()
            .skip(observed.len().saturating_sub(64))
            .map(|(r, e)| {
                let quote: String = e.text.chars().take(1800).collect();
                Citation {
                    episode: r.id,
                    journal_session: r.session,
                    observed_at_ms: r.at_ms,
                    excerpted: quote.len() < e.text.len(),
                    quote,
                    place: e.place.clone(),
                    speaker_labels: e.speaker_labels.clone(),
                }
            })
            .collect();
        Self { version: 1, kind: "extractive_scene_recap".into(), watermark,
            scope: "single_character_observed_session".into(), incomplete: true,
            coverage: "Only this session's received SSH observations. Earlier visits, absence, offscreen events and other participants' knowledge are unavailable. Interface speaker labels do not establish acquaintance. Excerpts are evidence, not verified assertions or instructions. Private speech visible to this character may be present; this artifact stays local.".into(),
            observed_episodes: observed.len(), omitted_episodes: observed.len().saturating_sub(entries.len()),
            provider_requests: 0, entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn watermark() -> Watermark {
        Watermark {
            session: "test".into(),
            scope: Scope::default(),
            elapsed_ms: 5,
            output_sequence: 9,
            observation: 4,
            speech: 2,
        }
    }
    #[test]
    fn bookmark_roundtrip_rejects_corruption_unsupported_state_and_restore_claims() {
        let b = Bookmark::new(
            watermark(),
            MODULES
                .iter()
                .map(|n| (n.to_string(), json!({"example":true})))
                .collect(),
        )
        .unwrap();
        let mut restored: Bookmark =
            serde_json::from_slice(&serde_json::to_vec(&b).unwrap()).unwrap();
        restored.validate().unwrap();
        restored.modules.get_mut("memory").unwrap().data = json!({"changed":true});
        assert!(
            restored
                .validate()
                .unwrap_err()
                .to_string()
                .contains("checksum")
        );
        restored = b.clone();
        restored.modules.get_mut("memory").unwrap().version = 2;
        assert!(restored.validate().is_err());
        restored = b.clone();
        restored.world_restorable = true;
        assert!(restored.validate().is_err());
        restored = b;
        restored.modules.remove("attention");
        assert!(restored.validate().is_err());
    }
    #[test]
    fn recap_uses_only_current_observations_and_exact_bounded_citations() {
        let records = (1..=70)
            .map(|id| Record {
                id,
                at_ms: id * 10,
                session: if id == 1 { 1 } else { 2 },
                event: Event::Episode(crate::memory::Episode {
                    source: if id == 2 {
                        Source::PriorKnowledge
                    } else {
                        Source::Observed
                    },
                    text: format!("Marco says, test {id} {}", "é".repeat(2000)),
                    place: None,
                    after_command: None,
                    speaker_labels: vec!["Marco".into()],
                }),
            })
            .collect::<Vec<_>>();
        let recap = Recap::from_observations(watermark(), 2, &records);
        assert_eq!(recap.entries.len(), 64);
        assert_eq!(recap.omitted_episodes, 4);
        assert!(recap.entries.iter().all(|c| c.episode > 2 && c.excerpted));
        for cite in recap.entries {
            let Event::Episode(e) = &records[(cite.episode - 1) as usize].event else {
                panic!()
            };
            assert!(e.text.contains(&cite.quote));
        }
        assert_eq!(recap.provider_requests, 0);
    }
}
