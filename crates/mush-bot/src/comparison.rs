//! Offline comparisons of validated captures. No provider, configuration, SSH,
//! or character state is consulted, and excluded lore text is never reconstructed.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use crate::{Result, composition::Contribution, experiment::revision, model::Capture};

const ESTIMATE_METHOD: &str = "UTF-8 content bytes / 3 rounded up; excludes provider framing; not a tokenizer or context-window guarantee";

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Include exact changed messages and setting values. The default report
    /// contains hashes, byte counts and provenance, without duplicating prompts.
    pub include_content: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub version: u32,
    /// Equality of the complete parsed request bodies, including message order,
    /// roles, content and model settings. JSON whitespace is not compared.
    pub request_equal: bool,
    pub messages_equal: bool,
    pub settings_equal: bool,
    /// Equality of composition metadata, independently of the request body.
    /// Two legacy captures both lack provenance and compare equal here.
    pub provenance_equal: bool,
    pub question_equal: bool,
    pub includes_content: bool,
    pub estimate_method: &'static str,
    pub before: Summary,
    pub after: Summary,
    pub settings: Vec<SettingChange>,
    /// Positional differences, not an edit script. Component movement is also
    /// identified separately by stable ID in the provenance comparison.
    pub messages: Vec<MessageChange>,
    pub composition: CompositionComparison,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub capture_version: u32,
    pub question: bool,
    pub request_revision: String,
    pub prompt_revision: String,
    pub composition_revision: Option<String>,
    pub message_count: usize,
    pub input_bytes: usize,
    pub estimated_input_tokens: usize,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
    Changed,
}

#[derive(Clone, Debug, Serialize)]
pub struct SettingChange {
    pub field: String,
    pub change: Change,
    pub before: Option<ValueSummary>,
    pub after: Option<ValueSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValueSummary {
    pub json_bytes: usize,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessageChange {
    pub index: usize,
    pub change: Change,
    pub changed_fields: Vec<String>,
    pub before: Option<MessageSummary>,
    pub after: Option<MessageSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessageSummary {
    pub role: String,
    pub component_id: Option<String>,
    pub bytes: usize,
    pub content_revision: String,
    /// Hashes the entire message object, so extra fields are not ignored.
    pub message_revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompositionComparison {
    pub before_available: bool,
    pub after_available: bool,
    /// None means at least one capture has no composition metadata.
    pub profile_equal: Option<bool>,
    pub changed_metadata: Vec<String>,
    pub unchanged_blocks: usize,
    pub blocks: Vec<BlockChange>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockChange {
    pub id: String,
    pub change: Change,
    /// True only for a block included on both sides at different message indexes.
    pub moved: bool,
    pub changed_fields: Vec<String>,
    pub before: Option<BlockSummary>,
    pub after: Option<BlockSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockSummary {
    pub provenance_index: usize,
    pub contribution: Contribution,
}

/// Compare two validated captures without copying their prompt content.
pub fn compare(before: &Capture, after: &Capture) -> Result<Report> {
    compare_with_options(before, after, Options::default())
}

/// Compare exact request data and provenance. Each input is limited to the
/// capture loader's 8 MB envelope; at most 179 message positions and 352 distinct
/// component IDs can appear. Full changed messages are included only on request.
pub fn compare_with_options(before: &Capture, after: &Capture, options: Options) -> Result<Report> {
    validate(before)?;
    validate(after)?;
    let before_messages = before.payload["messages"].as_array().unwrap();
    let after_messages = after.payload["messages"].as_array().unwrap();
    let before_settings = settings(before);
    let after_settings = settings(after);
    let mut setting_changes = vec![];
    for key in keys(&before_settings, &after_settings) {
        let left = before_settings.get(&key);
        let right = after_settings.get(&key);
        if left != right {
            setting_changes.push(SettingChange {
                field: key,
                change: change(left, right),
                before: left.map(|v| value_summary(v, options)).transpose()?,
                after: right.map(|v| value_summary(v, options)).transpose()?,
            });
        }
    }
    let mut message_changes = vec![];
    for index in 0..before_messages.len().max(after_messages.len()) {
        let left = before_messages.get(index);
        let right = after_messages.get(index);
        if left != right {
            message_changes.push(MessageChange {
                index,
                change: change(left, right),
                changed_fields: changed_fields(left, right),
                before: left
                    .map(|m| message_summary(before, m, index, options))
                    .transpose()?,
                after: right
                    .map(|m| message_summary(after, m, index, options))
                    .transpose()?,
            });
        }
    }
    Ok(Report {
        version: 1,
        request_equal: before.payload == after.payload,
        messages_equal: before_messages == after_messages,
        settings_equal: before_settings == after_settings,
        provenance_equal: serde_json::to_value(&before.composition)?
            == serde_json::to_value(&after.composition)?,
        question_equal: before.question == after.question,
        includes_content: options.include_content,
        estimate_method: ESTIMATE_METHOD,
        before: summary(before),
        after: summary(after),
        settings: setting_changes,
        messages: message_changes,
        composition: composition(before, after)?,
    })
}

fn validate(capture: &Capture) -> Result<()> {
    capture.validate()?;
    if serde_json::to_vec(capture)?.len() > 8_000_000 {
        return Err("capture exceeds comparison's 8 MB input limit".into());
    }
    // Older provenance validation did not reject duplicate IDs. Positional
    // request comparison is possible, but an ID-based explanation is ambiguous.
    if let Some(report) = &capture.composition {
        let mut ids = BTreeSet::new();
        if report.blocks.iter().any(|b| !ids.insert(&b.id)) {
            return Err("capture has duplicate composition IDs; comparison is ambiguous".into());
        }
    }
    Ok(())
}

fn summary(capture: &Capture) -> Summary {
    let messages = capture.payload["messages"].as_array().unwrap();
    let input_bytes: usize = messages
        .iter()
        .map(|m| m["content"].as_str().unwrap().len())
        .sum();
    Summary {
        capture_version: capture.version,
        question: capture.question,
        request_revision: capture.request_revision.clone(),
        prompt_revision: capture.prompt_revision.clone(),
        composition_revision: capture.composition_revision.clone(),
        message_count: messages.len(),
        input_bytes,
        estimated_input_tokens: input_bytes.div_ceil(3),
    }
}

fn settings(capture: &Capture) -> BTreeMap<String, Value> {
    capture
        .payload
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, _)| *k != "messages")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn keys<T>(left: &BTreeMap<String, T>, right: &BTreeMap<String, T>) -> BTreeSet<String> {
    left.keys().chain(right.keys()).cloned().collect()
}

fn change<T>(left: Option<T>, right: Option<T>) -> Change {
    match (left.is_some(), right.is_some()) {
        (false, true) => Change::Added,
        (true, false) => Change::Removed,
        _ => Change::Changed,
    }
}

fn changed_fields(left: Option<&Value>, right: Option<&Value>) -> Vec<String> {
    let left = left.and_then(Value::as_object);
    let right = right.and_then(Value::as_object);
    left.into_iter()
        .chain(right)
        .flat_map(|o| o.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|k| left.and_then(|o| o.get(*k)) != right.and_then(|o| o.get(*k)))
        .cloned()
        .collect()
}

fn value_summary(value: &Value, options: Options) -> Result<ValueSummary> {
    let bytes = serde_json::to_vec(value)?;
    Ok(ValueSummary {
        json_bytes: bytes.len(),
        revision: revision(&bytes),
        value: options.include_content.then(|| value.clone()),
    })
}

fn message_summary(
    capture: &Capture,
    message: &Value,
    index: usize,
    options: Options,
) -> Result<MessageSummary> {
    let content = message["content"].as_str().unwrap();
    Ok(MessageSummary {
        role: message["role"].as_str().unwrap().into(),
        component_id: capture.composition.as_ref().and_then(|report| {
            report
                .blocks
                .iter()
                .find(|b| b.message_index == Some(index))
                .map(|b| b.id.clone())
        }),
        bytes: content.len(),
        content_revision: revision(content.as_bytes()),
        message_revision: revision(&serde_json::to_vec(message)?),
        message: options.include_content.then(|| message.clone()),
    })
}

fn blocks(capture: &Capture) -> BTreeMap<String, BlockSummary> {
    capture
        .composition
        .iter()
        .flat_map(|r| r.blocks.iter().enumerate())
        .map(|(index, b)| {
            (
                b.id.clone(),
                BlockSummary {
                    provenance_index: index,
                    contribution: b.clone(),
                },
            )
        })
        .collect()
}

fn composition(before: &Capture, after: &Capture) -> Result<CompositionComparison> {
    let left_blocks = blocks(before);
    let right_blocks = blocks(after);
    let mut block_changes = vec![];
    let mut unchanged_blocks = 0;
    for id in keys(&left_blocks, &right_blocks) {
        let left = left_blocks.get(&id);
        let right = right_blocks.get(&id);
        let left_value = left
            .map(|b| serde_json::to_value(&b.contribution))
            .transpose()?;
        let right_value = right
            .map(|b| serde_json::to_value(&b.contribution))
            .transpose()?;
        let mut fields = changed_fields(left_value.as_ref(), right_value.as_ref());
        if left.map(|b| b.provenance_index) != right.map(|b| b.provenance_index) {
            fields.push("provenance_index".into());
        }
        if fields.is_empty() {
            unchanged_blocks += 1;
        } else {
            block_changes.push(BlockChange {
                id,
                change: change(left, right),
                moved: left.zip(right).is_some_and(|(l, r)| {
                    l.contribution
                        .message_index
                        .zip(r.contribution.message_index)
                        .is_some_and(|(l, r)| l != r)
                }),
                changed_fields: fields,
                before: left.cloned(),
                after: right.cloned(),
            });
        }
    }
    let metadata = |capture: &Capture| -> Result<Option<Value>> {
        capture
            .composition
            .as_ref()
            .map(|r| {
                let mut value = serde_json::to_value(r)?;
                value.as_object_mut().unwrap().remove("blocks");
                Ok(value)
            })
            .transpose()
    };
    Ok(CompositionComparison {
        before_available: before.composition.is_some(),
        after_available: after.composition.is_some(),
        profile_equal: before
            .composition
            .as_ref()
            .zip(after.composition.as_ref())
            .map(|(l, r)| l.profile_revision == r.profile_revision),
        changed_metadata: changed_fields(metadata(before)?.as_ref(), metadata(after)?.as_ref()),
        unchanged_blocks,
        blocks: block_changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::Settings;
    use serde_json::json;

    fn capture(settings: Option<Settings>) -> Capture {
        let (messages, composition) = if let Some(settings) = settings {
            let (messages, report) = settings
                .prepare(
                    "System instructions".into(),
                    &json!({"runner_goal":"Visit Marco", "memory":"I paid three coins."}),
                    None,
                )
                .unwrap();
            (messages, Some(report))
        } else {
            (
                vec![
                    json!({"role":"system", "content":"System instructions"}),
                    json!({"role":"user", "content":"Visit Marco"}),
                ],
                None,
            )
        };
        let payload = json!({"model":"local/a", "max_tokens":128,
            "temperature":0.25, "stream":false, "messages":messages});
        let mut capture = Capture {
            version: if composition.is_some() { 2 } else { 1 },
            question: false,
            prompt_revision: String::new(),
            request_revision: String::new(),
            payload,
            composition,
            composition_revision: None,
        };
        seal(&mut capture);
        capture.validate().unwrap();
        capture
    }

    fn seal(capture: &mut Capture) {
        capture.prompt_revision = revision(
            capture.payload["messages"][0]["content"]
                .as_str()
                .unwrap()
                .as_bytes(),
        );
        capture.request_revision = revision(&serde_json::to_vec(&capture.payload).unwrap());
        capture.composition_revision = capture
            .composition
            .as_ref()
            .map(|r| revision(&serde_json::to_vec(r).unwrap()));
    }

    #[test]
    fn model_settings_change_without_message_or_provenance_changes() {
        let before = capture(Some(Settings::default()));
        let mut after = before.clone();
        after.payload["model"] = json!("local/b");
        after.payload.as_object_mut().unwrap().remove("temperature");
        seal(&mut after);
        let report = compare(&before, &after).unwrap();
        assert!(!report.request_equal);
        assert!(!report.settings_equal);
        assert!(report.messages_equal && report.provenance_equal);
        assert!(report.messages.is_empty() && report.composition.blocks.is_empty());
        assert_eq!(report.settings.len(), 2);
        assert_eq!(report.settings[1].change, Change::Removed);
        assert!(report.settings[0].after.as_ref().unwrap().value.is_none());
        let verbose = compare_with_options(
            &before,
            &after,
            Options {
                include_content: true,
            },
        )
        .unwrap();
        assert_eq!(
            verbose.settings[0].after.as_ref().unwrap().value,
            Some(json!("local/b"))
        );
    }

    #[test]
    fn reordered_added_and_removed_blocks_have_stable_id_differences() {
        let before_settings: Settings = serde_json::from_value(json!({"components":[
            {"id":"goal","pointer":"/runner_goal","order":10},
            {"id":"memory","pointer":"/memory","order":20},
            {"id":"removed","text":"Old instructions","order":30}
        ]}))
        .unwrap();
        let after_settings: Settings = serde_json::from_value(json!({"components":[
            {"id":"goal","pointer":"/runner_goal","order":20},
            {"id":"memory","pointer":"/memory","order":10},
            {"id":"added","text":"New instructions","order":30}
        ]}))
        .unwrap();
        let before = capture(Some(before_settings));
        let after = capture(Some(after_settings));
        let report = compare_with_options(
            &before,
            &after,
            Options {
                include_content: true,
            },
        )
        .unwrap();
        assert!(!report.request_equal && !report.messages_equal && !report.provenance_equal);
        assert!(report.settings_equal);
        let block = |id| {
            report
                .composition
                .blocks
                .iter()
                .find(|b| b.id == id)
                .unwrap()
        };
        assert_eq!(block("added").change, Change::Added);
        assert_eq!(block("removed").change, Change::Removed);
        assert!(block("goal").moved && block("memory").moved);
        assert_eq!(report.messages[0].index, 1);
        assert_eq!(
            report.messages[0].after.as_ref().unwrap().message,
            Some(after.payload["messages"][1].clone())
        );
    }

    #[test]
    fn excluded_lore_changes_only_profile_provenance_without_disclosing_text() {
        let mut settings: Settings = serde_json::from_value(json!({"components":[],"lore":[
            {"id":"hidden","text":"SECRET_BEFORE","keywords":["Marco"]}
        ]}))
        .unwrap();
        let before = capture(Some(settings.clone()));
        settings.lore[0].text = "SECRET_AFTER".into();
        let after = capture(Some(settings.clone()));
        let report = compare_with_options(
            &before,
            &after,
            Options {
                include_content: true,
            },
        )
        .unwrap();
        assert!(report.request_equal && report.messages_equal && report.settings_equal);
        assert!(!report.provenance_equal);
        assert_eq!(report.composition.profile_equal, Some(false));
        assert_eq!(report.composition.changed_metadata, ["profile_revision"]);
        assert!(report.composition.blocks.is_empty());
        assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
        settings.lore[0].common_knowledge = true;
        let granted = capture(Some(settings));
        let report = compare(&after, &granted).unwrap();
        let block = &report.composition.blocks[0];
        assert_eq!(
            block.before.as_ref().unwrap().contribution.reason,
            "knowledge_not_granted"
        );
        assert_eq!(
            block.after.as_ref().unwrap().contribution.reason,
            "included"
        );
        assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
    }

    #[test]
    fn legacy_and_mixed_captures_compare_content_and_roles_exactly() {
        let before = capture(None);
        let report = compare(&before, &before).unwrap();
        assert!(report.request_equal && report.provenance_equal);
        assert!(!report.composition.before_available);
        let mut after = before.clone();
        after.payload["messages"][1]["content"] = json!("Visit Marco ");
        seal(&mut after);
        let changed = compare(&before, &after).unwrap();
        assert!(!changed.messages_equal);
        assert_eq!(changed.messages[0].changed_fields, ["content"]);
        let mut composed = capture(Some(Settings::default()));
        let mixed = compare(&before, &composed).unwrap();
        assert!(!mixed.provenance_equal && mixed.composition.after_available);
        assert_eq!(mixed.composition.profile_equal, None);
        let previous = composed.clone();
        composed.payload["messages"][1]["role"] = json!("system");
        seal(&mut composed);
        let changed = compare(&previous, &composed).unwrap();
        assert_eq!(changed.messages[0].changed_fields, ["role"]);
        assert!(!changed.messages_equal && changed.provenance_equal);
        let mut question = before.clone();
        question.question = true;
        question.payload["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"user","content":"Who is Marco?"}));
        seal(&mut question);
        let added = compare(&before, &question).unwrap();
        assert!(!added.question_equal);
        assert_eq!(added.messages[0].change, Change::Added);
        assert_eq!(
            compare(&question, &before).unwrap().messages[0].change,
            Change::Removed
        );
        let mut legacy_empty = before.clone();
        legacy_empty.payload["messages"][1]["content"] = json!("{}");
        seal(&mut legacy_empty);
        let composed_empty = capture(Some(
            serde_json::from_value(json!({"components":[]})).unwrap(),
        ));
        let same_request = compare(&legacy_empty, &composed_empty).unwrap();
        assert!(same_request.request_equal && !same_request.provenance_equal);
        // Reordering JSON object keys has no effect on the captured request.
        let roundtrip: Capture =
            serde_json::from_str(&serde_json::to_string_pretty(&before).unwrap()).unwrap();
        assert!(compare(&before, &roundtrip).unwrap().request_equal);
    }

    #[test]
    fn rejects_tampering_and_ambiguous_component_ids() {
        let before = capture(Some(Settings::default()));
        let mut changed = before.clone();
        changed.payload["model"] = json!("unsealed");
        assert!(compare(&before, &changed).is_err());
        let mut changed = before.clone();
        changed.composition.as_mut().unwrap().profile_revision = "unsealed".into();
        assert!(compare(&changed, &before).is_err());
        let mut changed = before.clone();
        let blocks = &mut changed.composition.as_mut().unwrap().blocks;
        blocks[1].id = blocks[0].id.clone();
        seal(&mut changed);
        assert!(
            compare(&changed, &before)
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
    }
}
