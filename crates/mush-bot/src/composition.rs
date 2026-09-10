//! Deterministic, inspectable prompt assembly. World data never runs local code.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Result, experiment::revision};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    /// Exact UTF-8 content budget; token counts below are explicitly estimates.
    pub max_input_bytes: usize,
    pub components: Vec<Component>,
    pub lore: Vec<Lore>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            max_input_bytes: 128_000,
            components: defaults(),
            lore: vec![],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Component {
    pub id: String,
    /// A JSON pointer into the actor's context. Mutually exclusive with text.
    pub pointer: Option<String>,
    /// Runner-authored instruction, never expanded as a template or script.
    pub text: Option<String>,
    pub enabled: bool,
    pub required: bool,
    pub order: i32,
    pub priority: i32,
    pub max_bytes: usize,
    pub purposes: Vec<String>,
}

impl Default for Component {
    fn default() -> Self {
        Self {
            id: String::new(),
            pointer: None,
            text: None,
            enabled: true,
            required: false,
            order: 100,
            priority: 0,
            max_bytes: 64_000,
            purposes: vec![],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lore {
    pub id: String,
    pub text: String,
    /// Explicitly authored common knowledge, or a grant to named characters.
    pub common_knowledge: bool,
    pub known_by: Vec<String>,
    /// Match utterance content and the current goal/question, never speaker labels.
    pub keywords: Vec<String>,
    pub purposes: Vec<String>,
    pub order: i32,
    pub priority: i32,
}

impl Default for Lore {
    fn default() -> Self {
        Self {
            id: String::new(),
            text: String::new(),
            common_knowledge: false,
            known_by: vec![],
            keywords: vec![],
            purposes: vec![],
            order: 500,
            priority: 0,
        }
    }
}

fn defaults() -> Vec<Component> {
    [
        "runner_role",
        "runner_goal",
        "runner_instructions",
        "decision_purpose",
        "group_affiliations",
        "character_profile",
        "character_state",
        "memory",
        "remembered_experience",
        "route_lookup",
        "reference_documents",
        "terminal_history",
        "recent_speech",
        "conversation_policy",
        "belief_feedback",
        "model_calls_remaining",
        "context_settings",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, field)| Component {
        id: field.into(),
        pointer: Some(format!("/{field}")),
        order: i as i32 * 10,
        required: matches!(
            field,
            "runner_role"
                | "runner_goal"
                | "runner_instructions"
                | "decision_purpose"
                | "recent_speech"
                | "model_calls_remaining"
                | "group_affiliations"
        ),
        priority: if field == "terminal_history" { -10 } else { 0 },
        ..Component::default()
    })
    .collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}

fn valid_purposes(purposes: &[String]) -> bool {
    purposes.len() <= 3
        && purposes
            .iter()
            .all(|p| ["goal", "conversation", "runner_question"].contains(&p.as_str()))
}

fn applies(purposes: &[String], purpose: &str) -> bool {
    purposes.is_empty() || purposes.iter().any(|p| p == purpose)
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !(4096..=1_000_000).contains(&self.max_input_bytes)
            || self.components.len() > 48
            || self.lore.len() > 128
        {
            return Err("composition needs version 1, 4096..1000000 input bytes, <=48 components and <=128 lore entries".into());
        }
        let mut ids = BTreeSet::new();
        for c in &self.components {
            if !valid_id(&c.id)
                || !ids.insert(c.id.clone())
                || c.pointer.is_some() == c.text.is_some()
                || c.pointer
                    .as_ref()
                    .is_some_and(|p| !p.starts_with('/') || p.len() > 200)
                || c.text
                    .as_ref()
                    .is_some_and(|t| t.is_empty() || t.len() > 64_000)
                || c.max_bytes == 0
                || c.max_bytes > 1_000_000
                || !valid_purposes(&c.purposes)
                || (c.required && (!c.enabled || !c.purposes.is_empty()))
            {
                return Err("invalid/duplicate prompt component; choose pointer or text and keep required components enabled for every purpose".into());
            }
        }
        for l in &self.lore {
            if !valid_id(&l.id)
                || !ids.insert(l.id.clone())
                || l.text.trim().is_empty()
                || l.text.len() > 32_000
                || l.known_by.len() > 64
                || l.keywords.len() > 32
                || l.known_by
                    .iter()
                    .chain(&l.keywords)
                    .any(|s| s.trim().is_empty() || s.len() > 200)
                || !valid_purposes(&l.purposes)
            {
                return Err(
                    "invalid/duplicate lore entry or bounded knowledge/keyword grants".into(),
                );
            }
        }
        Ok(())
    }

    pub fn prepare(
        &self,
        system: String,
        context: &Value,
        question: Option<&str>,
    ) -> Result<(Vec<Value>, Report)> {
        self.validate()?;
        let purpose = if question.is_some() {
            "runner_question"
        } else {
            context["decision_purpose"].as_str().unwrap_or("goal")
        };
        let mut messages = vec![json!({"role":"system", "content":system})];
        let mut used =
            messages[0]["content"].as_str().unwrap().len() + question.map_or(0, str::len);
        if used > self.max_input_bytes {
            return Err("system/question exceeds composition input budget".into());
        }
        let mut report = Report { version: 1, purpose: purpose.into(), profile_revision: revision(&serde_json::to_vec(self)?),
            input_bytes: 0, estimated_input_tokens: 0, estimate_method: "UTF-8 bytes / 3 rounded up; excludes provider framing; not a tokenizer or context-window guarantee".into(), blocks: vec![] };
        // Candidate allocation order is independent of the eventual message order.
        let mut candidates = Vec::new();
        for c in &self.components {
            let source = c
                .pointer
                .as_ref()
                .map_or_else(|| "runner_instruction".into(), |p| format!("context:{p}"));
            let mut entry = Contribution::new(&c.id, source);
            let value = c.pointer.as_ref().and_then(|p| context.pointer(p));
            if !c.enabled {
                entry.reason = "disabled".into();
            } else if !applies(&c.purposes, purpose) {
                entry.reason = "different_purpose".into();
            } else if c.pointer.is_some() && value.is_none_or(Value::is_null) {
                entry.reason = "missing_or_ablated".into();
            } else {
                let value = value.cloned().unwrap_or_else(|| json!(c.text));
                let mut content = json!({"component":c.id, "kind":if c.pointer.is_some() {"observed_or_authored_data"} else {"runner_instruction"}, "value":value}).to_string();
                if content.len() > c.max_bytes {
                    // Only raw terminal history is safely tail-trimmable. Evidence and
                    // structured speech are atomic so quotes/identities cannot be severed.
                    if c.pointer.as_deref() == Some("/terminal_history") && value.is_string() {
                        let overhead = content.len() - value.as_str().unwrap().len();
                        let clipped = crate::tail(
                            value.as_str().unwrap(),
                            c.max_bytes.saturating_sub(overhead),
                        );
                        content = json!({"component":c.id,"kind":"observed_or_authored_data","value":clipped}).to_string();
                        entry.truncated = true;
                    }
                    if content.len() > c.max_bytes || (c.required && entry.truncated) {
                        if c.required {
                            return Err(format!(
                                "required component {} exceeds its byte budget",
                                c.id
                            )
                            .into());
                        }
                        entry.reason = "component_budget".into();
                    }
                }
                if entry.reason.is_empty() {
                    candidates.push((
                        report.blocks.len(),
                        c.required,
                        c.priority,
                        c.order,
                        if c.pointer.is_some() {
                            "user"
                        } else {
                            "system"
                        },
                        content,
                    ));
                }
            }
            report.blocks.push(entry);
        }
        let mut scan = format!(
            "{} {}",
            context["runner_goal"].as_str().unwrap_or(""),
            question.unwrap_or("")
        );
        if let Some(utterances) = context["recent_speech"]["utterances"].as_array() {
            for utterance in utterances {
                if utterance["self_echo"] != true && utterance["kind"] != "ooc" {
                    scan.push('\n');
                    scan.push_str(utterance["text"].as_str().unwrap_or(""));
                }
            }
        }
        let scan = scan.to_lowercase();
        let name = context["character_profile"]["name"].as_str().unwrap_or("");
        for l in &self.lore {
            let mut entry = Contribution::new(&l.id, "authored_lore".into());
            if !l.common_knowledge && !l.known_by.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                entry.reason = "knowledge_not_granted".into();
            } else if !applies(&l.purposes, purpose) {
                entry.reason = "different_purpose".into();
            } else if !l.keywords.is_empty()
                && !l.keywords.iter().any(|k| scan.contains(&k.to_lowercase()))
            {
                entry.reason = "no_keyword_match".into();
            } else {
                entry.activation = Some(
                    if l.keywords.is_empty() {
                        "unconditional"
                    } else {
                        "keyword_match"
                    }
                    .into(),
                );
                candidates.push((
                    report.blocks.len(),
                    false,
                    l.priority,
                    l.order,
                    "user",
                    json!({"component":l.id,"kind":"authored_prior_knowledge","value":l.text})
                        .to_string(),
                ));
            }
            report.blocks.push(entry);
        }
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
        let mut selected = Vec::new();
        for (index, required, _, order, role, content) in candidates {
            if used + content.len() > self.max_input_bytes {
                if required {
                    return Err(format!(
                        "required component {} cannot fit composition input budget",
                        report.blocks[index].id
                    )
                    .into());
                }
                report.blocks[index].reason = "total_budget".into();
            } else {
                used += content.len();
                selected.push((order, index, role, content));
            }
        }
        selected.sort_by_key(|s| (s.0, s.1));
        for (_, index, role, content) in selected {
            let entry = &mut report.blocks[index];
            entry.reason = "included".into();
            entry.bytes = content.len();
            entry.content_revision = Some(revision(content.as_bytes()));
            entry.message_index = Some(messages.len());
            messages.push(json!({"role":role,"content":content}));
        }
        // Keep an explicit context message even in an all-disabled ablation.
        if messages.len() == 1 {
            messages.push(json!({"role":"user","content":"{}"}));
            used += 2;
        }
        if let Some(question) = question {
            messages.push(json!({"role":"user","content":question}));
        }
        if used > self.max_input_bytes {
            return Err("empty context exceeds composition budget".into());
        }
        report.input_bytes = used;
        report.estimated_input_tokens = used.div_ceil(3);
        Ok((messages, report))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u32,
    pub purpose: String,
    pub profile_revision: String,
    pub input_bytes: usize,
    pub estimated_input_tokens: usize,
    pub estimate_method: String,
    pub blocks: Vec<Contribution>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub id: String,
    pub source: String,
    pub reason: String,
    pub activation: Option<String>,
    pub truncated: bool,
    pub bytes: usize,
    pub content_revision: Option<String>,
    pub message_index: Option<usize>,
}

impl Contribution {
    fn new(id: &str, source: String) -> Self {
        Self {
            id: id.into(),
            source,
            reason: String::new(),
            activation: None,
            truncated: false,
            bytes: 0,
            content_revision: None,
            message_index: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lore_requires_a_knowledge_grant_and_scans_content_without_speaker_labels() {
        let settings: Settings = serde_json::from_value(json!({"components":[],"lore":[
            {"id":"private","text":"UNSEEN SECRET","keywords":["Marco"]},
            {"id":"friend","text":"Marco taught me to bind books.","known_by":["Rose"],"keywords":["Marco"]},
            {"id":"public","text":"The square is open to everyone.","common_knowledge":true}
        ]})).unwrap();
        let mut context = json!({"character_profile":{"name":"Rose"},"recent_speech":{"utterances":[
            {"speaker_label":"Marco","text":"Good morning.","self_echo":false,"kind":"say"}]}});
        let (messages, report) = settings.prepare("system".into(), &context, None).unwrap();
        assert!(!json!(messages).to_string().contains("Marco"));
        assert_eq!(report.blocks[0].reason, "knowledge_not_granted");
        assert_eq!(report.blocks[1].reason, "no_keyword_match");
        context["recent_speech"]["utterances"][0]["text"] = json!("I am Marco.");
        let (messages, report) = settings.prepare("system".into(), &context, None).unwrap();
        assert!(json!(messages).to_string().contains("taught me"));
        assert!(!json!(messages).to_string().contains("UNSEEN"));
        assert!(!json!(report).to_string().contains("UNSEEN"));
        context["character_profile"]["name"] = json!("Stranger");
        let (_, report) = settings
            .prepare("system".into(), &context, Some("Who is Marco?"))
            .unwrap();
        assert_eq!(report.blocks[1].reason, "knowledge_not_granted");
    }

    #[test]
    fn budgets_keep_required_speech_atomic_and_placement_is_independent_of_priority() {
        let mut settings: Settings =
            serde_json::from_value(json!({"max_input_bytes":4096,"components":[
                {"id":"old","pointer":"/terminal_history","max_bytes":20000,"priority":1000},
                {"id":"voice","text":"Speak plainly.","order":50},
                {"id":"speech","pointer":"/recent_speech","required":true,"order":100}
            ]}))
            .unwrap();
        let context = json!({"terminal_history":"x".repeat(8000),"recent_speech":{"utterances":[{"text":"Rose, can you help?"}]}});
        let first = settings.prepare("system".into(), &context, None).unwrap();
        let second = settings.prepare("system".into(), &context, None).unwrap();
        assert_eq!(json!(first), json!(second));
        assert_eq!(first.1.blocks[0].reason, "total_budget");
        assert_eq!(first.1.blocks[2].reason, "included");
        assert_eq!(first.1.blocks[1].message_index, Some(1));
        assert_eq!(first.1.blocks[2].message_index, Some(2));
        settings.components[2].max_bytes = 1;
        assert!(settings.prepare("system".into(), &context, None).is_err());
    }

    #[test]
    fn captures_preserve_both_legacy_requests_and_composed_provenance() {
        let model =
            serde_json::from_value(json!({"endpoint":"http://127.0.0.1","name":"test/model"}))
                .unwrap();
        let brain = crate::model::Brain::new(&model).unwrap();
        let mut prompts = crate::experiment::Prompts::default();
        let context = json!({"runner_goal":"Look around","decision_purpose":"goal","recent_speech":{"utterances":[]}});
        let legacy = brain.prepare(context.clone(), None, &prompts).unwrap();
        legacy.validate().unwrap();
        assert_eq!(legacy.version, 1);
        assert!(
            !json!(legacy)
                .as_object()
                .unwrap()
                .contains_key("composition")
        );
        prompts.composition = Some(Settings::default());
        let capture = brain
            .prepare(context, Some("Where am I?".into()), &prompts)
            .unwrap();
        assert_eq!(capture.version, 2);
        capture.validate().unwrap();
        let changed_model =
            serde_json::from_value(json!({"endpoint":"http://127.0.0.1","name":"test/other"}))
                .unwrap();
        let changed = capture.with_model(&changed_model).unwrap();
        changed.validate().unwrap();
        assert_eq!(capture.payload["messages"], changed.payload["messages"]);
        let mut tampered = capture;
        tampered.composition.as_mut().unwrap().blocks[0].source = "invented".into();
        assert!(tampered.validate().is_err());
    }
}
