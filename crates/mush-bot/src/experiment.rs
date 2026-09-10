//! Versioned runner-authored prompts and context controls. Never loaded from game text.
use std::{fs::File, io::Read, path::Path};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Result, tail};

pub fn revision(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn load_json<T: DeserializeOwned>(path: &Path, limit: usize) -> Result<T> {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("json"))
    {
        return Err("artifact extension must be .json".into());
    }
    let mut source = String::new();
    File::open(path)?
        .take((limit + 1) as u64)
        .read_to_string(&mut source)?;
    if source.len() > limit {
        return Err("JSON artifact exceeds its size limit".into());
    }
    Ok(serde_json::from_str(
        source.strip_prefix('\u{feff}').unwrap_or(&source),
    )?)
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("json"))
    {
        return Err("artifact extension must be .json".into());
    }
    let file = File::options().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(file, value)?;
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Prompts {
    pub version: u32,
    pub name: String,
    /// None uses the bundled prompt; Some replaces the full system message.
    pub decision_system: Option<String>,
    pub question_system: Option<String>,
    pub supplement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composition: Option<crate::composition::Settings>,
}

impl Default for Prompts {
    fn default() -> Self {
        Self {
            version: 1,
            name: "bundled".into(),
            decision_system: None,
            question_system: None,
            supplement: String::new(),
            composition: None,
        }
    }
}

impl Prompts {
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = load_json(path, 128_000)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(composition) = &self.composition {
            composition.validate()?;
        }
        if self.version != 1
            || self.name.trim().is_empty()
            || self.name.len() > 200
            || self.name.chars().any(char::is_control)
            || [&self.decision_system, &self.question_system]
                .iter()
                .any(|s| s.as_ref().is_some_and(|s| s.trim().is_empty()))
            || serde_json::to_vec(self)?.len() > 128_000
        {
            return Err("invalid prompt profile: version 1, name, and nonempty replacement prompts required (128 KB maximum)".into());
        }
        Ok(())
    }

    pub fn system(&self, question: bool) -> String {
        let replacement = if question {
            &self.question_system
        } else {
            &self.decision_system
        };
        let mut system = replacement
            .clone()
            .unwrap_or_else(|| crate::model::Brain::system(question));
        if !self.supplement.is_empty() {
            system.push_str("\n\n");
            system.push_str(&self.supplement);
        }
        system
    }

    pub fn effective(&self) -> Value {
        serde_json::json!({"profile":self, "decision_system":self.system(false), "question_system":self.system(true),
            "decision_revision":revision(self.system(false).as_bytes()), "question_revision":revision(self.system(true).as_bytes())})
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Context {
    pub version: u32,
    pub history_bytes: usize,
    pub speech_limit: usize,
    pub speech_bytes: usize,
    pub recall_bytes: usize,
    pub recall: crate::memory::RecallSettings,
    pub retrieval: bool,
    pub routes: bool,
    pub references: bool,
    pub working_memory: bool,
    pub character: bool,
    /// None retains all authored examples; an explicit limit selects by context relevance.
    pub voice_examples: Option<usize>,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            version: 1,
            history_bytes: 1_000_000,
            speech_limit: 64,
            speech_bytes: 24_000,
            recall_bytes: 128_000,
            recall: Default::default(),
            retrieval: true,
            routes: true,
            references: true,
            working_memory: true,
            character: true,
            voice_examples: None,
        }
    }
}

impl Context {
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = load_json(path, 16_000)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<()> {
        self.recall.validate()?;
        if self.version != 1
            || self.history_bytes > 1_000_000
            || self.speech_limit > 64
            || self.speech_bytes > 24_000
            || self.recall_bytes > 128_000
            || self.voice_examples.is_some_and(|n| n > 64)
        {
            return Err("context version must be 1; history <= 1000000, speech_limit <= 64, speech_bytes <= 24000, recall_bytes <= 128000, voice_examples <= 64".into());
        }
        Ok(())
    }

    pub fn apply(&self, context: &mut Value) {
        if let Some(history) = context["terminal_history"].as_str() {
            context["terminal_history"] = Value::String(tail(history, self.history_bytes).into());
        }
        for (enabled, field) in [
            (self.retrieval, "remembered_experience"),
            (self.routes, "route_lookup"),
            (self.references, "reference_documents"),
            (self.working_memory, "memory"),
            (self.character, "character_profile"),
        ] {
            if !enabled {
                context[field] = Value::Null;
            }
        }
        // Filter speech before deriving any other context from it.
        // Speech layout belongs to Attention; its control metadata remains visible.
        if let Some(items) = context["recent_speech"]["utterances"].as_array_mut() {
            let mut bytes = 0;
            let keep = items
                .iter()
                .rev()
                .take(self.speech_limit)
                .take_while(|item| {
                    bytes += item.to_string().len();
                    bytes <= self.speech_bytes
                })
                .count();
            items.drain(..items.len().saturating_sub(keep));
        }
        if let Some(limit) = self.voice_examples {
            let speech = context["recent_speech"]["utterances"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let query = format!(
                "{} {}",
                context["runner_goal"].as_str().unwrap_or(""),
                speech
            );
            let words: std::collections::BTreeSet<_> = query
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 2)
                .map(str::to_lowercase)
                .collect();
            if let Some(examples) = context
                .pointer_mut("/character_profile/voice/examples")
                .and_then(Value::as_array_mut)
            {
                let mut ranked: Vec<_> = examples
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let situation = e["situation"].as_str().unwrap_or("").to_lowercase();
                        let score = words
                            .iter()
                            .filter(|w| situation.contains(w.as_str()))
                            .count();
                        (std::cmp::Reverse(score), i)
                    })
                    .collect();
                ranked.sort();
                ranked.truncate(limit);
                // Preserve authored ordering after selecting the relevant subset.
                ranked.sort_by_key(|(_, i)| *i);
                *examples = ranked
                    .into_iter()
                    .map(|(_, i)| examples[i].clone())
                    .collect();
            }
        }
        context["context_settings"] = serde_json::to_value(self).expect("serializable settings");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn voice_selection_keeps_relevant_examples_without_mutating_the_sheet_or_undoing_ablations() {
        let source = json!({"runner_goal":"Decline an invitation","character_profile":{"voice":{"examples":[
            {"situation":"greeting","line":"Morning."}, {"situation":"decline an invitation","line":"Another time, perhaps."}]}},
            "recent_speech":{"utterances":[]}});
        let mut view = source.clone();
        let mut settings = Context {
            voice_examples: Some(1),
            ..Context::default()
        };
        settings.apply(&mut view);
        assert_eq!(
            view["character_profile"]["voice"]["examples"][0]["line"],
            "Another time, perhaps."
        );
        assert_eq!(
            source["character_profile"]["voice"]["examples"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        settings.character = false;
        settings.apply(&mut view);
        assert!(view["character_profile"].is_null());
        settings.character = true;
        let mut view = source.clone();
        view["runner_goal"] = json!("");
        view["recent_speech"]["utterances"] = json!([{"text":"decline an invitation"}]);
        let mut unfiltered = view.clone();
        settings.apply(&mut unfiltered);
        assert_eq!(
            unfiltered["character_profile"]["voice"]["examples"][0]["line"],
            "Another time, perhaps."
        );
        settings.speech_limit = 0;
        settings.apply(&mut view);
        assert_eq!(
            view["character_profile"]["voice"]["examples"][0]["line"],
            "Morning."
        );
    }

    #[test]
    fn ablations_filter_only_the_supplied_view_and_preserve_authored_inputs() {
        let settings:Context=serde_json::from_value(json!({"history_bytes":4,"speech_limit":1,"retrieval":false,"routes":false,"references":false,"working_memory":false})).unwrap();
        let source = json!({"terminal_history":"older 🌹", "memory":"remembered prices", "remembered_experience":{"records":[1,2]},"route_lookup":{"exit":"east"},"reference_documents":"help",
            "recent_speech":{"utterances":[{"text":"old"},{"text":"latest"}],"snapshot":2}});
        let mut context = source.clone();
        settings.apply(&mut context);
        assert_eq!(context["terminal_history"], "🌹");
        assert!(context["memory"].is_null());
        assert!(context["remembered_experience"].is_null());
        assert_eq!(
            context["recent_speech"]["utterances"],
            json!([{"text":"latest"}])
        );
        assert_eq!(source["memory"], "remembered prices");
        assert!(serde_json::from_value::<Context>(json!({"retrival":false})).is_err());
        assert!(
            serde_json::from_value::<Context>(json!({"speech_limit":65}))
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn prompt_hashes_cover_exact_effective_text_and_replacements_are_explicit() {
        let defaults = Prompts::default();
        assert_eq!(defaults.system(false), crate::model::Brain::system(false));
        let edited = Prompts {
            decision_system: Some("Decide carefully.".into()),
            supplement: "Use prices.".into(),
            ..Prompts::default()
        };
        assert_eq!(edited.system(false), "Decide carefully.\n\nUse prices.");
        assert_ne!(
            edited.effective()["decision_revision"],
            defaults.effective()["decision_revision"]
        );
        assert!(edited.system(true).starts_with("Answer the runner"));
        assert!(
            Prompts {
                decision_system: Some("".into()),
                ..defaults
            }
            .validate()
            .is_err()
        );
    }
}
