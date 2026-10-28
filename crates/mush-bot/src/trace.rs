//! Bounded offline inspection of recorded bot events. Identifier associations are
//! explicit, one-hop links within an actor; they do not establish causality.
use std::{
    collections::BTreeSet,
    fmt,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};

use crate::{Result, experiment};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ROW_BYTES: usize = 4 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Joined,
    Standalone,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    /// One-based physical source line, never a reordered or filtered position.
    pub line: usize,
    pub actor: Option<String>,
    pub kind: String,
    /// Joined harness elapsed time, or an explicit standalone session clock.
    /// Never inferred from wall timestamps or compared across clock domains.
    pub elapsed_ms: Option<u64>,
    pub timestamp_ms: Option<u64>,
    pub seq: Option<u64>,
    pub request_id: Option<u64>,
    pub command_sequence: Option<u64>,
    pub raw: Value,
}

#[derive(Debug)]
pub struct Trace {
    pub source: PathBuf,
    /// SHA-256 of the exact source bytes, including whitespace and newlines.
    pub revision: String,
    pub format: Format,
    pub events: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub version: u32,
    pub actors: Vec<String>,
    pub kinds: Vec<String>,
    pub contains: Option<String>,
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub offset: usize,
    pub limit: usize,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            version: 1,
            actors: Vec::new(),
            kinds: Vec::new(),
            contains: None,
            from_ms: None,
            to_ms: None,
            offset: 0,
            limit: 200,
        }
    }
}

impl Query {
    pub fn load(path: &Path) -> Result<Self> {
        let query: Self = experiment::load_json(path, 64 * 1024)?;
        query.validate()?;
        Ok(query)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err("trace query version must be 1".into());
        }
        for (name, values) in [("actors", &self.actors), ("kinds", &self.kinds)] {
            if values.len() > 64
                || values.iter().any(|value| {
                    value.trim().is_empty()
                        || value.len() > 256
                        || value.chars().any(char::is_control)
                })
                || values.iter().collect::<BTreeSet<_>>().len() != values.len()
            {
                return Err(format!(
                    "trace query {name} must contain at most 64 unique nonempty names of at most 256 bytes without control characters"
                )
                .into());
            }
        }
        if self
            .contains
            .as_ref()
            .is_some_and(|text| text.is_empty() || text.len() > 4096)
        {
            return Err("trace query contains must be 1..4096 bytes when supplied".into());
        }
        if self.from_ms.zip(self.to_ms).is_some_and(|(a, b)| a > b) {
            return Err("trace query from_ms must not exceed to_ms".into());
        }
        if self.offset > MAX_EVENTS || !(1..=1000).contains(&self.limit) {
            return Err("trace query offset must be <= 100000 and limit must be 1..1000".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Limits {
    bytes: usize,
    row_bytes: usize,
    events: usize,
}

const LIMITS: Limits = Limits {
    bytes: MAX_BYTES,
    row_bytes: MAX_ROW_BYTES,
    events: MAX_EVENTS,
};

fn line_error(path: &Path, line: usize, message: &str) -> crate::Error {
    format!("trace {} line {line}: {message}", path.display()).into()
}

/// Read one explicitly named JSONL artifact. No directory discovery, application
/// configuration, provider access, or partial recovery occurs here.
pub fn load(path: &Path) -> Result<Trace> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
    {
        return Err("trace artifact extension must be .jsonl".into());
    }
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err("trace source must be a regular .jsonl file".into());
    }
    if metadata.len() > MAX_BYTES as u64 {
        return Err(line_error(path, 1, "file exceeds the 64 MiB size limit"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    parse(path, &bytes, LIMITS)
}

fn integer(
    object: &Map<String, Value>,
    field: &str,
    nullable: bool,
    required: bool,
) -> std::result::Result<Option<u64>, String> {
    match object.get(field) {
        None if !required => Ok(None),
        None => Err(format!("missing {field}")),
        Some(Value::Null) if nullable => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            format!(
                "{field} must be an unsigned 64-bit integer{}",
                if nullable { " or null" } else { "" }
            )
        }),
    }
}

fn name(object: &Map<String, Value>, field: &str) -> std::result::Result<String, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "{field} must be a nonempty string of at most 256 bytes without control characters"
            )
        })
}

fn entry(raw: Value, line: usize) -> std::result::Result<(Format, Entry), String> {
    let row = raw.as_object().ok_or("row must be a JSON object")?;
    let joined = ["event", "actor"]
        .iter()
        .any(|field| row.contains_key(*field));
    let (format, actor, elapsed_ms, event) = if joined {
        if [
            "kind",
            "data",
            "seq",
            "timestamp_ms",
            "request_id",
            "command_sequence",
        ]
        .iter()
        .any(|field| row.contains_key(*field))
        {
            return Err("row mixes joined envelope and standalone event fields".into());
        }
        (
            Format::Joined,
            Some(name(row, "actor")?),
            integer(row, "elapsed_ms", false, true)?,
            row.get("event")
                .and_then(Value::as_object)
                .ok_or("joined event must be a JSON object")?,
        )
    } else {
        (
            Format::Standalone,
            None,
            integer(row, "elapsed_ms", false, false)?,
            row,
        )
    };
    if ["event", "actor"]
        .iter()
        .any(|field| event.contains_key(*field))
    {
        return Err("event contains joined envelope fields".into());
    }
    let kind = name(event, "kind")?;
    // Validate an actor's explicit clock even when the harness envelope owns
    // the time used for joined filtering.
    integer(event, "elapsed_ms", false, false)?;
    if !event.contains_key("data") {
        return Err("event is missing data".into());
    }
    let timestamp_ms = integer(event, "timestamp_ms", false, false)?;
    let seq = integer(event, "seq", false, false)?;
    let request_id = integer(event, "request_id", true, false)?;
    let command_sequence = integer(event, "command_sequence", true, false)?;
    Ok((
        format,
        Entry {
            line,
            actor,
            kind,
            elapsed_ms,
            timestamp_ms,
            seq,
            request_id,
            command_sequence,
            raw,
        },
    ))
}

/// A validation-only pass avoids choosing a last value for duplicate keys before
/// extracting actor and request identifiers, while leaving `Value` construction
/// to `serde_json` so 64-bit integer payloads are not rounded through `f64`.
struct UniqueJson;

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct UniqueVisitor;

        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = ();

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }

            fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
                Ok(())
            }
            fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
                Ok(())
            }
            fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
                Ok(())
            }
            fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
                Ok(())
            }
            fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
                Ok(())
            }
            fn visit_unit<E>(self) -> std::result::Result<(), E> {
                Ok(())
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> std::result::Result<(), A::Error> {
                while sequence.next_element::<UniqueJson>()?.is_some() {}
                Ok(())
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
                let mut keys = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                    map.next_value::<UniqueJson>()?;
                }
                Ok(())
            }
        }

        deserializer.deserialize_any(UniqueVisitor).map(|()| Self)
    }
}

fn parse(path: &Path, bytes: &[u8], limits: Limits) -> Result<Trace> {
    if bytes.len() > limits.bytes {
        return Err(line_error(path, 1, "file exceeds the 64 MiB size limit"));
    }
    let revision = experiment::revision(bytes);
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let mut events = Vec::new();
    let mut format = None;
    let mut rows = bytes.split(|byte| *byte == b'\n').peekable();
    let mut line = 0;
    while let Some(row) = rows.next() {
        line += 1;
        // A final newline terminates the preceding record; it is not a blank row.
        if row.is_empty() && rows.peek().is_none() {
            break;
        }
        if row.len() > limits.row_bytes {
            return Err(line_error(path, line, "row exceeds the 4 MiB size limit"));
        }
        if events.len() >= limits.events {
            return Err(line_error(
                path,
                line,
                "trace exceeds the 100000 event limit",
            ));
        }
        let text = std::str::from_utf8(row)
            .map_err(|_| line_error(path, line, "row is not valid UTF-8"))?;
        if text.trim().is_empty() {
            return Err(line_error(path, line, "blank rows are not JSON events"));
        }
        serde_json::from_str::<UniqueJson>(text).map_err(|error| {
            if error.is_data() {
                line_error(path, line, "duplicate object key")
            } else {
                line_error(
                    path,
                    line,
                    &format!("invalid JSON at column {}", error.column()),
                )
            }
        })?;
        let raw: Value = serde_json::from_str(text).map_err(|error| {
            line_error(
                path,
                line,
                &format!("invalid JSON at column {}", error.column()),
            )
        })?;
        let (row_format, event) =
            entry(raw, line).map_err(|error| line_error(path, line, &error))?;
        if format.is_some_and(|expected| expected != row_format) {
            return Err(line_error(
                path,
                line,
                "joined and standalone formats cannot be mixed",
            ));
        }
        format = Some(row_format);
        events.push(event);
    }
    let format = format.ok_or_else(|| line_error(path, 1, "trace contains no events"))?;
    Ok(Trace {
        source: path.to_owned(),
        revision,
        format,
        events,
    })
}

impl Trace {
    /// All matching source indices before pagination, preserving recorded order.
    /// Name filters are exact; contains is a lowercase literal search over the
    /// serialized original row, including metadata and escaped JSON strings.
    pub fn matching(&self, query: &Query) -> Result<Vec<usize>> {
        query.validate()?;
        let contains = query.contains.as_ref().map(|text| text.to_lowercase());
        let mut indices = Vec::new();
        for (index, event) in self.events.iter().enumerate() {
            if !query.actors.is_empty()
                && !event
                    .actor
                    .as_ref()
                    .is_some_and(|actor| query.actors.contains(actor))
            {
                continue;
            }
            if !query.kinds.is_empty() && !query.kinds.contains(&event.kind) {
                continue;
            }
            if (query.from_ms.is_some() || query.to_ms.is_some())
                && !event.elapsed_ms.is_some_and(|time| {
                    query.from_ms.is_none_or(|start| time >= start)
                        && query.to_ms.is_none_or(|end| time <= end)
                })
            {
                continue;
            }
            if let Some(needle) = &contains
                && !serde_json::to_string(&event.raw)?
                    .to_lowercase()
                    .contains(needle)
            {
                continue;
            }
            indices.push(index);
        }
        Ok(indices)
    }

    /// Explicit matching IDs within the same actor, including the anchor when
    /// it has an ID. This is one-hop association, never inferred causality.
    pub fn related(&self, index: usize) -> Vec<usize> {
        let Some(anchor) = self.events.get(index) else {
            return Vec::new();
        };
        self.events
            .iter()
            .enumerate()
            .filter(|(_, event)| {
                event.actor == anchor.actor
                    && (anchor.request_id.is_some() && event.request_id == anchor.request_id
                        || anchor.command_sequence.is_some()
                            && event.command_sequence == anchor.command_sequence)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Produce a bounded page. Payloads are absent unless explicitly requested;
    /// filtering may inspect content without exporting that content.
    pub fn report(&self, query: &Query, include_content: bool) -> Result<Value> {
        let matching = self.matching(query)?;
        let entries = matching
            .iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|index| {
                let event = &self.events[*index];
                let mut value = json!({
                    "line":event.line,"actor":event.actor,"kind":event.kind,
                    "elapsed_ms":event.elapsed_ms,"timestamp_ms":event.timestamp_ms,
                    "seq":event.seq,"request_id":event.request_id,
                    "command_sequence":event.command_sequence
                });
                if include_content {
                    value
                        .as_object_mut()
                        .expect("metadata is an object")
                        .insert("raw".into(), event.raw.clone());
                }
                value
            })
            .collect::<Vec<_>>();
        let next_offset = (query.offset.saturating_add(entries.len()) < matching.len())
            .then_some(query.offset + entries.len());
        Ok(json!({
            "version":1,
            "source":{"path":self.source,"sha256":self.revision,"format":self.format},
            "query":query,
            "totals":{"events":self.events.len(),"matched":matching.len(),"returned":entries.len()},
            "pagination":{"offset":query.offset,"limit":query.limit,"next_offset":next_offset},
            "include_content":include_content,
            "entries":entries,
            "limitations":[
                "Recorded order and explicit identifiers are preserved; identifier association does not imply causality.",
                "Elapsed time exists only in joined envelopes; standalone timestamps are not converted to elapsed time.",
                "The source hash covers exact file bytes; included raw rows are parsed JSON, not original formatting.",
                "Trace records are observations and debugging evidence, not independent proof of an outcome."
            ]
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_session_clock_does_not_turn_a_standalone_trace_into_a_joined_trace() {
        let row = serde_json::json!({"kind":"speech-prepared","data":{},"elapsed_ms":42,"provenance":{"session":"one"}});
        let standalone = trace(&row.to_string());
        assert_eq!(standalone.format, Format::Standalone);
        assert_eq!(standalone.events[0].elapsed_ms, Some(42));
        let joined =
            trace(&serde_json::json!({"actor":"a","elapsed_ms":81,"event":row}).to_string());
        assert_eq!(joined.events[0].elapsed_ms, Some(81));
        assert_eq!(joined.events[0].raw["event"]["elapsed_ms"], 42);
    }

    fn trace(source: &str) -> Trace {
        parse(Path::new("trace.jsonl"), source.as_bytes(), LIMITS).unwrap()
    }

    fn joined(actor: &str, time: u64, event: Value) -> Value {
        json!({"actor":actor,"elapsed_ms":time,"event":event})
    }

    fn rows(rows: &[Value]) -> String {
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn reads_current_log_and_joined_shapes_without_reordering_or_invented_clocks() {
        let source = concat!(
            "\u{feff}{\"seq\":2,\"timestamp_ms\":100,\"request_id\":1,\"command_sequence\":null,\"kind\":\"future-kind\",\"data\":{\"private\":\"value\"}}\r\n",
            "{\"seq\":1,\"timestamp_ms\":90,\"request_id\":null,\"command_sequence\":2,\"kind\":\"send\",\"data\":\"look\"}\n"
        );
        let parsed = trace(source);
        assert_eq!(parsed.format, Format::Standalone);
        assert_eq!(parsed.revision, experiment::revision(source.as_bytes()));
        assert_eq!(
            parsed
                .events
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![Some(2), Some(1)]
        );
        assert_eq!(parsed.events[1].line, 2);
        assert!(
            parsed
                .events
                .iter()
                .all(|entry| entry.elapsed_ms.is_none() && entry.actor.is_none())
        );
        let source = rows(&[
            joined(
                "_harness",
                20,
                json!({"kind":"phase-start","data":{"id":"phase-1"}}),
            ),
            joined(
                "mira",
                10,
                json!({"kind":"game","data":"speech","seq":1,"timestamp_ms":100,"request_id":null,"command_sequence":null}),
            ),
        ]);
        let parsed = trace(&source);
        assert_eq!(parsed.format, Format::Joined);
        assert_eq!(parsed.events[0].elapsed_ms, Some(20));
        assert_eq!(parsed.events[1].elapsed_ms, Some(10));
        assert_eq!(
            parsed.events[0].raw,
            serde_json::from_str::<Value>(source.lines().next().unwrap()).unwrap()
        );
    }

    #[test]
    fn rejects_malformed_tails_mixed_formats_and_bad_numbers_without_content() {
        let valid = r#"{"kind":"game","data":"ok"}"#;
        for (tail, expected) in [
            (r#"{"kind":"game","data":"SECRET""#, "invalid JSON"),
            (r#"{"kind":null,"data":"SECRET"}"#, "kind must"),
            (r#"{"kind":"game"}"#, "missing data"),
            (r#"{"kind":"game","data":"SECRET","seq":null}"#, "seq must"),
            (r#"{"kind":"game","data":"SECRET","seq":1.0}"#, "seq must"),
            (
                r#"{"kind":"game","data":"SECRET","timestamp_ms":-1}"#,
                "timestamp_ms must",
            ),
            (
                r#"{"kind":"game","data":"SECRET","request_id":"2"}"#,
                "request_id must",
            ),
            (
                r#"{"kind":"game","data":"SECRET","command_sequence":18446744073709551616}"#,
                "command_sequence must",
            ),
            (
                r#"{"actor":"a","elapsed_ms":1,"event":{"kind":"game","data":"SECRET"}}"#,
                "formats cannot be mixed",
            ),
            (" ", "blank rows"),
        ] {
            let error = parse(
                Path::new("case.jsonl"),
                format!("{valid}\n{tail}").as_bytes(),
                LIMITS,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("case.jsonl line 2:"), "{error}");
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("SECRET"), "{error}");
        }
        for source in [
            "",
            "\n",
            "[]",
            r#"{"actor":"a","elapsed_ms":null,"event":{"kind":"game","data":null}}"#,
            r#"{"actor":"a","elapsed_ms":1,"event":{"kind":"game","data":null},"kind":"game"}"#,
        ] {
            assert!(parse(Path::new("bad.jsonl"), source.as_bytes(), LIMITS).is_err());
        }
        assert!(
            parse(
                Path::new("bad.jsonl"),
                b"{\"kind\":\"game\",\"data\":\"\xff\"}",
                LIMITS
            )
            .unwrap_err()
            .to_string()
            .contains("line 1: row is not valid UTF-8")
        );
    }

    #[test]
    fn rejects_recursive_duplicate_keys_without_rounding_precise_payload_numbers() {
        let valid = r#"{"kind":"game","data":null}"#;
        for row in [
            r#"{"actor":"SECRET_A","actor":"SECRET_B","elapsed_ms":1,"event":{"kind":"game","data":null}}"#,
            r#"{"actor":"SECRET_A","\u0061ctor":"SECRET_B","elapsed_ms":1,"event":{"kind":"game","data":null}}"#,
            r#"{"kind":"model","request_id":1,"request_id":2,"data":"SECRET"}"#,
            r#"{"kind":"model","request_id":1,"request_\u0069d":2,"data":"SECRET"}"#,
            r#"{"actor":"mira","elapsed_ms":1,"event":{"kind":"model","request_id":1,"request_id":2,"data":null}}"#,
            r#"{"kind":"game","data":{"records":[{"SECRET":1,"SECRET":2}]}}"#,
        ] {
            let error = parse(
                Path::new("duplicate.jsonl"),
                format!("{valid}\n{row}").as_bytes(),
                LIMITS,
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("duplicate.jsonl line 2: duplicate object key"),
                "{error}"
            );
            assert!(!error.contains("SECRET"), "{error}");
            assert!(!error.contains("request_id"), "{error}");
        }
        let source = r#"{"kind":"future","data":{"integer":18446744073709551615,"signed":-9223372036854775808,"decimal":"0.12345678901234567890123456789","values":[null,true,false,0,-3,1.5,1e300,"text"]}}"#;
        let parsed = trace(source);
        assert_eq!(
            parsed.events[0].raw["data"]["integer"].to_string(),
            "18446744073709551615"
        );
        assert_eq!(
            parsed.events[0].raw["data"]["signed"].to_string(),
            "-9223372036854775808"
        );
        assert_eq!(
            parsed.events[0].raw["data"]["decimal"].as_str(),
            Some("0.12345678901234567890123456789")
        );
        assert_eq!(
            parsed.events[0].raw,
            serde_json::from_str::<Value>(source).unwrap()
        );
    }

    #[test]
    fn bounded_parser_fails_whole_artifact_instead_of_returning_partial_events() {
        let row = r#"{"kind":"game","data":null}"#;
        let small = Limits {
            bytes: 200,
            row_bytes: 100,
            events: 1,
        };
        assert!(
            parse(Path::new("x.jsonl"), &[b' '; 201], small)
                .unwrap_err()
                .to_string()
                .contains("file exceeds")
        );
        assert!(
            parse(Path::new("x.jsonl"), &[b' '; 101], small)
                .unwrap_err()
                .to_string()
                .contains("row exceeds")
        );
        assert!(
            parse(
                Path::new("x.jsonl"),
                format!("{row}\n{row}").as_bytes(),
                small
            )
            .unwrap_err()
            .to_string()
            .contains("line 2: trace exceeds")
        );
        assert_eq!(
            parse(Path::new("x.jsonl"), row.as_bytes(), small)
                .unwrap()
                .events
                .len(),
            1
        );
        assert!(
            load(Path::new("trace.json"))
                .unwrap_err()
                .to_string()
                .contains(".jsonl")
        );
    }

    #[test]
    fn filters_are_exact_inclusive_and_combined_before_pagination() {
        let parsed = trace(&rows(&[
            joined("mira", 5, json!({"kind":"game","data":"A Rose"})),
            joined("marco", 10, json!({"kind":"game","data":"a rose"})),
            joined("mira", 15, json!({"kind":"send","data":"a ROSE"})),
            joined("mira", 20, json!({"kind":"game","data":"a rose"})),
            joined("mira", 25, json!({"kind":"game","data":"garden"})),
        ]));
        let query = Query {
            actors: vec!["mira".into()],
            kinds: vec!["game".into(), "send".into()],
            contains: Some("ROSE".into()),
            from_ms: Some(5),
            to_ms: Some(20),
            offset: 1,
            limit: 1,
            ..Default::default()
        };
        assert_eq!(parsed.matching(&query).unwrap(), vec![0, 2, 3]);
        let report = parsed.report(&query, false).unwrap();
        assert_eq!(
            report["totals"],
            json!({"events":5,"matched":3,"returned":1})
        );
        assert_eq!(report["entries"][0]["line"], 3);
        assert_eq!(report["pagination"]["next_offset"], 2);
        let exact = Query {
            actors: vec!["Mira".into()],
            ..Default::default()
        };
        assert!(parsed.matching(&exact).unwrap().is_empty());
        let standalone = trace(r#"{"kind":"game","timestamp_ms":12,"data":"rose"}"#);
        assert!(
            standalone
                .matching(&Query {
                    from_ms: Some(0),
                    ..Default::default()
                })
                .unwrap()
                .is_empty()
        );
        assert!(
            parsed
                .report(
                    &Query {
                        offset: 99,
                        ..Default::default()
                    },
                    false
                )
                .unwrap()["pagination"]["next_offset"]
                .is_null()
        );
    }

    #[test]
    fn metadata_reports_omit_payloads_and_only_explicit_content_exports_raw_rows() {
        let parsed = trace(
            r#"{"kind":"model","data":{"prompt":"VERY_PRIVATE","answer":"SECRET"},"request_id":2}"#,
        );
        let metadata = parsed.report(&Query::default(), false).unwrap();
        let serialized = metadata.to_string();
        assert!(!serialized.contains("VERY_PRIVATE"));
        assert!(!serialized.contains("SECRET"));
        assert!(metadata["entries"][0].get("raw").is_none());
        assert_eq!(metadata["source"]["sha256"], parsed.revision);
        assert_eq!(
            parsed.report(&Query::default(), true).unwrap()["entries"][0]["raw"],
            parsed.events[0].raw
        );
    }

    #[test]
    fn associations_use_one_hop_explicit_ids_and_never_cross_actors_or_null_ids() {
        let parsed = trace(&rows(&[
            joined(
                "mira",
                0,
                json!({"kind":"a","data":null,"request_id":1,"command_sequence":4}),
            ),
            joined(
                "mira",
                1,
                json!({"kind":"b","data":null,"request_id":1,"command_sequence":5}),
            ),
            joined(
                "mira",
                2,
                json!({"kind":"c","data":null,"request_id":2,"command_sequence":4}),
            ),
            joined(
                "mira",
                3,
                json!({"kind":"d","data":null,"request_id":3,"command_sequence":5}),
            ),
            joined(
                "marco",
                4,
                json!({"kind":"e","data":null,"request_id":1,"command_sequence":4}),
            ),
            joined(
                "mira",
                5,
                json!({"kind":"f","data":null,"request_id":null,"command_sequence":null}),
            ),
            joined("mira", 6, json!({"kind":"g","data":null})),
        ]));
        assert_eq!(parsed.related(0), vec![0, 1, 2]);
        assert!(parsed.related(5).is_empty());
        assert!(parsed.related(6).is_empty());
        assert!(parsed.related(100).is_empty());
        let standalone = trace(&rows(&[
            json!({"kind":"a","data":null,"request_id":0}),
            json!({"kind":"b","data":null,"request_id":0}),
        ]));
        assert_eq!(standalone.related(0), vec![0, 1]);
    }

    #[test]
    fn query_rejects_unknown_fields_duplicate_names_and_invalid_bounds() {
        assert!(serde_json::from_str::<Query>(r#"{"regex":".*"}"#).is_err());
        for query in [
            Query {
                version: 2,
                ..Default::default()
            },
            Query {
                actors: vec!["mira".into(), "mira".into()],
                ..Default::default()
            },
            Query {
                kinds: vec!["".into()],
                ..Default::default()
            },
            Query {
                kinds: vec!["game\n".into()],
                ..Default::default()
            },
            Query {
                contains: Some(String::new()),
                ..Default::default()
            },
            Query {
                contains: Some("x".repeat(4097)),
                ..Default::default()
            },
            Query {
                from_ms: Some(2),
                to_ms: Some(1),
                ..Default::default()
            },
            Query {
                offset: 100_001,
                ..Default::default()
            },
            Query {
                limit: 0,
                ..Default::default()
            },
            Query {
                limit: 1001,
                ..Default::default()
            },
        ] {
            assert!(query.validate().is_err());
        }
        Query::default().validate().unwrap();
        assert_eq!(serde_json::from_str::<Query>("{}").unwrap().limit, 200);
    }
}
