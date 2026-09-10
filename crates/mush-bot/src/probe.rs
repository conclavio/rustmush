//! Deterministic answer checks against the evidence in one exact model request.
//! No journal lookup, model judge, external state or generated summary is used.
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};
use serde_json::{Value, json};

use crate::{
    Result,
    experiment::{load_json, revision},
    memory::{Event, Record, Source},
    model::Capture,
    organization::identifier,
};

const MAX_JSON_BYTES: usize = 128_000;
const MAX_DIAGNOSTICS: usize = 32;
const MAX_CITATIONS: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u32,
    pub id: String,
    pub question: String,
    pub fields: Vec<Field>,
    pub checks: Vec<Check>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: FieldType,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    String,
    Number,
    Boolean,
}

impl FieldType {
    fn accepts(self, value: &Value) -> bool {
        value.is_null()
            || match self {
                Self::String => value.as_str().is_some_and(|s| s.len() <= 8000),
                Self::Number => value.is_number(),
                Self::Boolean => value.is_boolean(),
            }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub field: String,
    pub equals: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceCheck>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvidenceCheck {
    pub source: Option<Source>,
    /// Case-sensitive substring required in the cited quote, not elsewhere in its episode.
    pub contains: Option<String>,
    /// Count distinct episode IDs for this field, not repeated quotations.
    pub min_citations: usize,
}

impl Default for EvidenceCheck {
    fn default() -> Self {
        Self {
            source: None,
            contains: None,
            min_citations: 1,
        }
    }
}

impl Spec {
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = load_json(path, MAX_JSON_BYTES)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !identifier(&self.id)
            || self.question.trim().is_empty()
            || self.question.len() > 12_000
            || !(1..=32).contains(&self.fields.len())
            || !(1..=64).contains(&self.checks.len())
            || serde_json::to_vec(self)?.len() > MAX_JSON_BYTES
        {
            return Err("probe requires version 1, a bounded ID/question, 1..32 fields, 1..64 checks and at most 128000 JSON bytes".into());
        }
        let mut fields = BTreeMap::new();
        for field in &self.fields {
            if !identifier(&field.name)
                || field.description.trim().is_empty()
                || field.description.len() > 2000
                || fields.insert(field.name.as_str(), field.kind).is_some()
            {
                return Err(
                    "probe fields need unique bounded names and descriptions of 1..2000 bytes"
                        .into(),
                );
            }
        }
        let mut checks = BTreeSet::new();
        for check in &self.checks {
            if !identifier(&check.id)
                || !checks.insert(check.id.as_str())
                || fields
                    .get(check.field.as_str())
                    .is_none_or(|kind| !kind.accepts(&check.equals))
                || check.evidence.as_ref().is_some_and(|e| {
                    !(1..=16).contains(&e.min_citations)
                        || e.contains
                            .as_ref()
                            .is_some_and(|text| text.trim().is_empty() || text.len() > 4000)
                })
            {
                return Err("probe checks need unique IDs, a defined field, a matching primitive or null, and bounded evidence constraints".into());
            }
        }
        Ok(())
    }

    /// Only the question and output schema enter the request. Checks, expected
    /// values, evidence predicates and the definition ID remain runner-side.
    pub fn question_prompt(&self) -> Result<String> {
        self.validate()?;
        let task = json!({"question":self.question,"fields":self.fields});
        Ok(format!(
            "Answer the following structured question using the character's supplied knowledge. Return ONLY one JSON object, without Markdown or additional prose, with exactly two keys: answers and evidence. answers is an object containing every defined field and no other fields. Use the specified primitive type or null when the answer is unestablished; do not guess. evidence is an array of objects with exactly field (a defined field name), record_id (a positive integer), and quote (a nonempty exact excerpt). Cite only episode records actually present in remembered_experience in this request, never working notes, belief records, external IDs, or another character's history. Each field may cite an episode ID only once. Use an empty evidence array when no qualifying episode is supplied. A quote is provenance, not proof that a speaker's claim is true. Preserve the distinction between observed events, reported claims, attempted commands, and authored prior knowledge. Do not provide private reasoning.\n\n{task}"
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grade {
    pub version: u32,
    pub probe_id: String,
    pub definition_revision: String,
    pub request_revision: String,
    pub success: bool,
    pub response_valid: bool,
    pub checks: Vec<CheckGrade>,
    pub invalid_citations: Vec<InvalidCitation>,
    pub invalid_citations_omitted: usize,
    pub format_errors: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckGrade {
    pub id: String,
    pub passed: bool,
    pub reason: String,
    pub evidence_ids: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvalidCitation {
    pub index: usize,
    pub record_id: Option<u64>,
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    #[serde(deserialize_with = "unique_answers")]
    answers: BTreeMap<String, Value>,
    // Individual malformed citations can be diagnosed without copying their text.
    #[serde(deserialize_with = "unique_evidence")]
    evidence: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Citation {
    field: String,
    record_id: u64,
    quote: String,
}

fn unique_answers<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, Value>, D::Error> {
    struct AnswersVisitor;
    impl<'de> Visitor<'de> for AnswersVisitor {
        type Value = BTreeMap<String, Value>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object of unique answer fields")
        }
        fn visit_map<M: MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, Value>()? {
                if result.len() >= 32 || result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate or too many answer fields",
                    ));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(AnswersVisitor)
}

// serde_json::Value normally overwrites repeated object keys. Keep malformed
// citation diagnostics while preventing a duplicate field/ID/quote from being
// silently replaced before Citation's strict schema is checked.
fn unique_evidence<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Value>, D::Error> {
    struct Object(Value);
    impl<'de> Deserialize<'de> for Object {
        fn deserialize<D: Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct ObjectVisitor;
            impl<'de> Visitor<'de> for ObjectVisitor {
                type Value = Object;
                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a citation object with unique keys")
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<Self::Value, M::Error> {
                    let mut result = serde_json::Map::new();
                    while let Some((key, value)) = map.next_entry::<String, Value>()? {
                        if result.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate citation key"));
                        }
                    }
                    Ok(Object(Value::Object(result)))
                }
            }
            deserializer.deserialize_map(ObjectVisitor)
        }
    }
    Vec::<Object>::deserialize(deserializer)
        .map(|items| items.into_iter().map(|item| item.0).collect())
}

/// Only context-owned remembered_experience records qualify. Raw terminal text,
/// references and lore strings are never recursively parsed for episode-shaped JSON.
fn episodes(capture: &Capture) -> Result<BTreeMap<u64, Record>> {
    let messages = capture.payload["messages"]
        .as_array()
        .expect("validated capture");
    let mut views = Vec::new();
    if capture.version == 1 {
        if let Ok(context) = serde_json::from_str::<Value>(messages[1]["content"].as_str().unwrap())
        {
            views.push(context["remembered_experience"].clone());
        }
    } else if let Some(composition) = &capture.composition {
        for block in &composition.blocks {
            if block.source != "context:/remembered_experience" || block.reason != "included" {
                continue;
            }
            let Some(index) = block.message_index else {
                continue;
            };
            let message = &messages[index];
            if message["role"] != "user" {
                continue;
            }
            let Ok(wrapper) = serde_json::from_str::<Value>(message["content"].as_str().unwrap())
            else {
                continue;
            };
            if wrapper["component"] == block.id && wrapper["kind"] == "observed_or_authored_data" {
                views.push(wrapper["value"].clone());
            }
        }
    }
    let mut records = BTreeMap::new();
    for view in views {
        if let Some(items) = view["records"].as_array() {
            for item in items {
                let Ok(record) = serde_json::from_value::<Record>(item.clone()) else {
                    continue;
                };
                if record.id == 0 || !matches!(record.event, Event::Episode(_)) {
                    continue;
                }
                if let Some(previous) = records.get(&record.id)
                    && serde_json::to_value(previous)? != serde_json::to_value(&record)?
                {
                    return Err(
                        "probe capture contains conflicting evidence with the same record ID"
                            .into(),
                    );
                }
                records.insert(record.id, record);
            }
        }
    }
    Ok(records)
}

fn reject(grade: &mut Grade, reason: &str) {
    grade.response_valid = false;
    if grade.format_errors.len() < MAX_DIAGNOSTICS
        && !grade.format_errors.iter().any(|old| old == reason)
    {
        grade.format_errors.push(reason.into());
    }
}

fn invalid_citation(grade: &mut Grade, index: usize, record_id: Option<u64>, reason: &str) {
    grade.response_valid = false;
    if grade.invalid_citations.len() < MAX_DIAGNOSTICS {
        grade.invalid_citations.push(InvalidCitation {
            index,
            record_id,
            reason: reason.into(),
        });
    } else {
        grade.invalid_citations_omitted += 1;
    }
}

/// Grade exact values and citation structure, not semantic entailment or world truth.
/// Invalid definitions/captures return Err; invalid model responses return a failed Grade.
pub fn grade(spec: &Spec, capture: &Capture, raw_response: &str) -> Result<Grade> {
    spec.validate()?;
    capture.validate()?;
    let records = episodes(capture)?;
    let mut grade = Grade {
        version: 1,
        probe_id: spec.id.clone(),
        definition_revision: revision(&serde_json::to_vec(spec)?),
        request_revision: capture.request_revision.clone(),
        success: false,
        response_valid: true,
        checks: spec
            .checks
            .iter()
            .map(|check| CheckGrade {
                id: check.id.clone(),
                passed: false,
                reason: "invalid_response".into(),
                evidence_ids: vec![],
            })
            .collect(),
        invalid_citations: vec![],
        invalid_citations_omitted: 0,
        format_errors: vec![],
    };
    if raw_response.len() > MAX_JSON_BYTES {
        reject(&mut grade, "response_exceeds_128000_bytes");
        return Ok(grade);
    }
    let answer: Answer = match serde_json::from_str(raw_response) {
        Ok(answer) => answer,
        Err(_) => {
            reject(&mut grade, "invalid_json_or_response_schema");
            return Ok(grade);
        }
    };
    if answer.answers.len() != spec.fields.len()
        || answer
            .answers
            .keys()
            .any(|key| !spec.fields.iter().any(|field| field.name == *key))
    {
        reject(&mut grade, "missing_or_unexpected_answer_fields");
    }
    for field in &spec.fields {
        match answer.answers.get(&field.name) {
            None => reject(&mut grade, "missing_or_unexpected_answer_fields"),
            Some(value) if !field.kind.accepts(value) => {
                reject(&mut grade, "answer_type_or_size_mismatch")
            }
            _ => {}
        }
    }
    if answer.evidence.len() > MAX_CITATIONS {
        reject(&mut grade, "more_than_128_citations");
        return Ok(grade);
    }
    let mut seen = BTreeSet::new();
    let mut citations = Vec::new();
    for (index, value) in answer.evidence.into_iter().enumerate() {
        let record_id = value["record_id"].as_u64();
        let citation: Citation = match serde_json::from_value(value) {
            Ok(citation) => citation,
            Err(_) => {
                invalid_citation(&mut grade, index, record_id, "invalid_citation_schema");
                continue;
            }
        };
        let reason = if !spec.fields.iter().any(|field| field.name == citation.field) {
            Some("undefined_citation_field")
        } else if citation.record_id == 0
            || citation.quote.trim().is_empty()
            || citation.quote.len() > 4000
        {
            Some("invalid_citation_id_or_quote_size")
        } else if !seen.insert((citation.field.clone(), citation.record_id)) {
            Some("duplicate_field_record_citation")
        } else {
            match records.get(&citation.record_id) {
                None => Some("episode_not_in_prepared_capture"),
                Some(Record {
                    event: Event::Episode(episode),
                    ..
                }) if !episode.text.contains(&citation.quote) => {
                    Some("quote_not_in_captured_episode")
                }
                _ => None,
            }
        };
        if let Some(reason) = reason {
            invalid_citation(&mut grade, index, Some(citation.record_id), reason);
        } else {
            citations.push(citation);
        }
    }
    for (check, result) in spec.checks.iter().zip(&mut grade.checks) {
        let qualifying: BTreeSet<_> = citations
            .iter()
            .filter(|citation| {
                if citation.field != check.field {
                    return false;
                }
                let Some(constraint) = &check.evidence else {
                    return true;
                };
                let Some(Record {
                    event: Event::Episode(episode),
                    ..
                }) = records.get(&citation.record_id)
                else {
                    return false;
                };
                constraint
                    .source
                    .as_ref()
                    .is_none_or(|source| source == &episode.source)
                    && constraint
                        .contains
                        .as_ref()
                        .is_none_or(|needle| citation.quote.contains(needle))
            })
            .map(|citation| citation.record_id)
            .collect();
        result.evidence_ids = qualifying.iter().copied().collect();
        result.reason = if !grade.response_valid {
            "invalid_response"
        } else if answer.answers.get(&check.field) != Some(&check.equals) {
            "answer_mismatch"
        } else if check
            .evidence
            .as_ref()
            .is_some_and(|e| qualifying.len() < e.min_citations)
        {
            "insufficient_matching_citations"
        } else {
            result.passed = true;
            "matched"
        }
        .into();
    }
    grade.success = grade.response_valid && grade.checks.iter().all(|check| check.passed);
    Ok(grade)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{composition, memory::Episode};

    fn spec() -> Spec {
        serde_json::from_value(json!({
            "version":1,"id":"rose_price","question":"What price did you hear, and is a purchase confirmed?",
            "fields":[
                {"name":"price","type":"number","description":"Numeric quoted amount, or null if unestablished."},
                {"name":"currency","type":"string","description":"Currency of the quoted price."},
                {"name":"purchase_confirmed","type":"boolean","description":"Whether direct purchase confirmation is supplied."},
                {"name":"merchant","type":"string","description":"Known name of the merchant, or null if unestablished."}
            ],
            "checks":[
                {"id":"price","field":"price","equals":7,"evidence":{"source":"observed","contains":"seven silver"}},
                {"id":"currency","field":"currency","equals":"silver"},
                {"id":"purchase","field":"purchase_confirmed","equals":false},
                {"id":"unknown_merchant","field":"merchant","equals":null}
            ]
        })).unwrap()
    }

    fn episode(id: u64, source: Source, text: &str) -> Record {
        Record {
            id,
            at_ms: 100,
            session: 1,
            event: Event::Episode(Episode {
                source,
                text: text.into(),
                place: None,
                after_command: None,
                speaker_labels: vec!["Marco".into()],
            }),
        }
    }

    fn prepared(context: Value, composition: Option<composition::Settings>) -> Capture {
        let system = "Question answering instructions.";
        let (messages, report) = if let Some(settings) = composition {
            let (messages, report) = settings
                .prepare(system.into(), &context, Some("Question"))
                .unwrap();
            (messages, Some(report))
        } else {
            (
                vec![
                    json!({"role":"system","content":system}),
                    json!({"role":"user","content":context.to_string()}),
                    json!({"role":"user","content":"Question"}),
                ],
                None,
            )
        };
        let payload =
            json!({"messages":messages,"model":"local-test","max_tokens":1000,"stream":false});
        let capture = Capture {
            version: if report.is_some() { 2 } else { 1 },
            question: true,
            prompt_revision: revision(system.as_bytes()),
            request_revision: revision(&serde_json::to_vec(&payload).unwrap()),
            payload,
            composition_revision: report
                .as_ref()
                .map(|r| revision(&serde_json::to_vec(r).unwrap())),
            composition: report,
        };
        capture.validate().unwrap();
        capture
    }

    fn with_episode(version: u32, source: Source, text: &str) -> Capture {
        prepared(
            json!({"remembered_experience":{"records":[episode(7,source,text)]}}),
            (version == 2).then(composition::Settings::default),
        )
    }

    fn answer() -> Value {
        json!({"answers":{"price":7,"currency":"silver","purchase_confirmed":false,"merchant":null},
            "evidence":[{"field":"price","record_id":7,"quote":"The florist quoted seven silver for a rose."}]})
    }

    #[test]
    fn grades_exact_primitives_null_and_captured_citations_in_legacy_and_composed_requests() {
        let spec = spec();
        for version in [1, 2] {
            let capture = with_episode(
                version,
                Source::Observed,
                "Marco says, \"The florist quoted seven silver for a rose.\"",
            );
            let result = grade(&spec, &capture, &answer().to_string()).unwrap();
            assert!(result.response_valid && result.success);
            assert_eq!(result.checks[0].evidence_ids, vec![7]);
            assert_eq!(result.request_revision, capture.request_revision);
            assert_eq!(
                result.definition_revision,
                revision(&serde_json::to_vec(&spec).unwrap())
            );
            let mut incorrect = answer();
            incorrect["answers"]["price"] = json!(8);
            let result = grade(&spec, &capture, &incorrect.to_string()).unwrap();
            assert!(result.response_valid && !result.success);
            assert_eq!(result.checks[0].reason, "answer_mismatch");
            assert!(result.checks[1..].iter().all(|check| check.passed));

            let mut incorrect = answer();
            incorrect["answers"]["merchant"] = json!("Marco");
            let result = grade(&spec, &capture, &incorrect.to_string()).unwrap();
            assert!(!result.success && !result.checks[3].passed);
            // A transcript label does not make an unknown merchant's name correct.
            assert!(!serde_json::to_string(&result).unwrap().contains("Marco"));
        }
    }

    #[test]
    fn questions_never_include_expected_values_or_private_evidence_predicates() {
        let mut spec = spec();
        spec.id = "PRIVATE_DEFINITION_ID".into();
        spec.checks[1].equals = json!("PRIVATE_EXPECTED_CURRENCY");
        spec.checks[0].evidence.as_mut().unwrap().contains =
            Some("PRIVATE_REQUIRED_EXCERPT".into());
        let prompt = spec.question_prompt().unwrap();
        assert!(prompt.contains(&spec.question));
        assert!(prompt.contains("purchase_confirmed"));
        assert!(!prompt.contains("PRIVATE_"));
        assert!(!prompt.contains("min_citations"));
        spec.checks[1].equals = json!("ANOTHER_EXPECTED_VALUE");
        assert_eq!(spec.question_prompt().unwrap(), prompt);
    }

    #[test]
    fn rejects_fabricated_absent_duplicate_or_foreign_episode_citations_without_echoing_text() {
        let spec = spec();
        let capture = with_episode(
            1,
            Source::Observed,
            "The florist quoted seven silver for a rose.",
        );
        let mut absent = answer();
        absent["evidence"][0]["record_id"] = json!(9999);
        let result = grade(&spec, &capture, &absent.to_string()).unwrap();
        assert!(!result.response_valid && !result.success);
        assert_eq!(
            result.invalid_citations[0].reason,
            "episode_not_in_prepared_capture"
        );
        let mut invented = answer();
        invented["evidence"][0]["quote"] = json!("PRIVATE_FABRICATED_QUOTE");
        let result = grade(&spec, &capture, &invented.to_string()).unwrap();
        assert_eq!(
            result.invalid_citations[0].reason,
            "quote_not_in_captured_episode"
        );
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("PRIVATE_FABRICATED_QUOTE")
        );
        let other = with_episode(2, Source::Observed, "The mayor mentioned a chess match.");
        assert!(
            !grade(&spec, &other, &answer().to_string())
                .unwrap()
                .response_valid,
            "matching record IDs in another request cannot authorize a missing quote"
        );
        let mut duplicate = answer();
        let citation = duplicate["evidence"][0].clone();
        duplicate["evidence"].as_array_mut().unwrap().push(citation);
        let result = grade(&spec, &capture, &duplicate.to_string()).unwrap();
        assert_eq!(
            result.invalid_citations[0].reason,
            "duplicate_field_record_citation"
        );
        assert!(!result.success);
    }

    #[test]
    fn source_contains_and_distinct_citation_counts_are_structural_constraints() {
        let mut spec = spec();
        let capture = with_episode(
            1,
            Source::Sent,
            "The florist quoted seven silver for a rose.",
        );
        let result = grade(&spec, &capture, &answer().to_string()).unwrap();
        assert!(result.response_valid && !result.success);
        assert_eq!(result.checks[0].reason, "insufficient_matching_citations");
        spec.checks[0].evidence.as_mut().unwrap().source = Some(Source::Sent);
        assert!(
            grade(&spec, &capture, &answer().to_string())
                .unwrap()
                .success,
            "the evaluator compares the authored predicate; it does not promote an attempt to confirmed truth"
        );
        spec.checks[0].evidence.as_mut().unwrap().contains =
            Some("not in the cited excerpt".into());
        assert!(
            !grade(&spec, &capture, &answer().to_string())
                .unwrap()
                .success
        );
        spec.checks[0].evidence.as_mut().unwrap().contains = None;
        spec.checks[0].evidence.as_mut().unwrap().min_citations = 2;
        assert!(
            !grade(&spec, &capture, &answer().to_string())
                .unwrap()
                .success
        );
        let capture = prepared(
            json!({"remembered_experience":{"records":[
                episode(7,Source::Sent,"The florist quoted seven silver for a rose."),
                episode(8,Source::Sent,"Another quoted offer.")
            ]}}),
            None,
        );
        let mut two = answer();
        two["evidence"]
            .as_array_mut()
            .unwrap()
            .push(json!({"field":"price","record_id":8,"quote":"Another quoted offer."}));
        let result = grade(&spec, &capture, &two.to_string()).unwrap();
        assert!(result.success);
        assert_eq!(result.checks[0].evidence_ids, vec![7, 8]);
    }

    #[test]
    fn citations_cannot_escape_retrieval_ablations_through_lore_terminal_text_or_beliefs() {
        let fake = json!({"records":[episode(7,Source::Observed,"The florist quoted seven silver for a rose.")]});
        let context = json!({"remembered_experience":null,"terminal_history":fake.to_string(),"reference_documents":fake,"memory":fake});
        let mut settings = composition::Settings::default();
        settings.lore.push(composition::Lore {
            id: "lore".into(),
            text: fake.to_string(),
            common_knowledge: true,
            ..Default::default()
        });
        for capture in [
            prepared(context.clone(), None),
            prepared(context, Some(settings)),
        ] {
            let result = grade(&spec(), &capture, &answer().to_string()).unwrap();
            assert!(!result.response_valid);
            assert_eq!(
                result.invalid_citations[0].reason,
                "episode_not_in_prepared_capture"
            );
        }
        let mut record = serde_json::to_value(episode(
            7,
            Source::Observed,
            "The florist quoted seven silver for a rose.",
        ))
        .unwrap();
        record["event"] =
            json!({"type":"working_notes","data":"The florist quoted seven silver for a rose."});
        let capture = prepared(json!({"remembered_experience":{"records":[record]}}), None);
        assert!(
            !grade(&spec(), &capture, &answer().to_string())
                .unwrap()
                .response_valid
        );
    }

    #[test]
    fn strict_response_schema_and_limits_reject_ambiguous_or_unbounded_answers() {
        let spec = spec();
        let capture = with_episode(
            1,
            Source::Observed,
            "The florist quoted seven silver for a rose.",
        );
        for raw in ["not JSON".into(),format!("```json\n{}\n```",answer()),
            format!("{} {}",answer(),answer())," ".repeat(MAX_JSON_BYTES + 1),
            r#"{"answers":{"price":7,"price":8},"evidence":[]}"#.into(),
            r#"{"answers":{},"evidence":[{"field":"price","record_id":7,"record_id":8,"quote":"x"}]}"#.into()]
        {
            let result = grade(&spec,&capture,&raw).unwrap();
            assert!(!result.response_valid && !result.success);
            assert!(!result.format_errors.is_empty());
        }
        for mutate in [0, 1, 2, 3, 4] {
            let mut response = answer();
            match mutate {
                0 => {
                    response["answers"]
                        .as_object_mut()
                        .unwrap()
                        .remove("merchant");
                }
                1 => response["answers"]["extra"] = json!("PRIVATE_EXTRA_FIELD"),
                2 => response["answers"]["price"] = json!("7"),
                3 => response["answers"]["currency"] = json!(["silver"]),
                _ => response["unexpected"] = json!(true),
            }
            assert!(
                !grade(&spec, &capture, &response.to_string())
                    .unwrap()
                    .response_valid
            );
        }
        let mut response = answer();
        response["evidence"] = json!(
            (0..100)
                .map(|n| json!({"field":"price","record_id":1000+n,"quote":"PRIVATE_INVALID"}))
                .collect::<Vec<_>>()
        );
        let result = grade(&spec, &capture, &response.to_string()).unwrap();
        assert_eq!(result.invalid_citations.len(), 32);
        assert_eq!(result.invalid_citations_omitted, 68);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("PRIVATE_INVALID")
        );
        response["evidence"] = json!(
            (0..129)
                .map(|n| json!({"field":"price","record_id":1000+n,"quote":"x"}))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            grade(&spec, &capture, &response.to_string())
                .unwrap()
                .format_errors,
            vec!["more_than_128_citations"]
        );
    }

    #[test]
    fn rejects_duplicate_or_incompatible_definition_fields_checks_and_modified_captures() {
        let original = spec();
        for case in 0..7 {
            let mut spec = original.clone();
            match case {
                0 => spec.fields.push(spec.fields[0].clone()),
                1 => spec.checks.push(spec.checks[0].clone()),
                2 => spec.checks[0].field = "undefined".into(),
                3 => spec.checks[0].equals = json!("7"),
                4 => spec.checks[0].evidence.as_mut().unwrap().min_citations = 0,
                5 => spec.checks.clear(),
                _ => spec.version = 2,
            }
            assert!(spec.validate().is_err());
        }
        let mut capture = with_episode(
            1,
            Source::Observed,
            "The florist quoted seven silver for a rose.",
        );
        capture.payload["messages"][1]["content"] = json!("Modified without its request checksum");
        assert!(grade(&original, &capture, &answer().to_string()).is_err());
        let conflicting = prepared(
            json!({"remembered_experience":{"records":[
                episode(7,Source::Observed,"first source"),episode(7,Source::Observed,"conflicting source")
            ]}}),
            None,
        );
        assert!(grade(&original, &conflicting, &answer().to_string()).is_err());
    }
}
