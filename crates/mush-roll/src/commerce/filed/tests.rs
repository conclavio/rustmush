use std::collections::BTreeMap;

use mush_core::{ClockRef, ClockStamp, ObjId};
use serde_json::{Value, json};

use super::*;
use crate::{Envelope, RevisionRef};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn time(at: i64) -> Result<WorldIcSeconds, super::super::ValueError> {
    WorldIcSeconds::from_stamp(&ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    })
}

fn revision(number: u64) -> Result<Revision, crate::RollError> {
    let source = format!("private executable {number}");
    let envelope = Envelope {
        schema: json!({}),
        presets: json!({}),
        profiles: json!({"Default": {}}),
        rules: json!({}),
        declared_class: "I".into(),
        mind_abi: "test=1".into(),
    };
    Ok(Revision {
        format: crate::FORMAT,
        provenance: None,
        reference: RevisionRef {
            key: RevisionKey {
                character: ObjId(7),
                mind: 1,
                number,
            },
            digest: envelope.digest(&source)?,
        },
        source: crate::source_digest(&source),
        envelope,
        notes: "private notes".into(),
    })
}

#[derive(Default)]
struct Book {
    ready: bool,
    lines: BTreeMap<LineId, CharterId>,
    revisions: BTreeMap<RevisionKey, Revision>,
    contents: BTreeMap<FiledRevisionId, FiledRevisionContent>,
}

impl ContentLookup for Book {
    fn check_available(&self) -> Result<(), ContentError> {
        if self.ready {
            Ok(())
        } else {
            Err(ContentError::Unavailable)
        }
    }

    fn line_charter(&self, line: LineId) -> Result<Option<CharterId>, ContentError> {
        Ok(self.lines.get(&line).copied())
    }

    fn revision(&self, key: RevisionKey) -> Result<Option<&Revision>, ContentError> {
        Ok(self.revisions.get(&key))
    }

    fn filed(&self, id: FiledRevisionId) -> Result<Option<&FiledRevisionContent>, ContentError> {
        Ok(self.contents.get(&id))
    }
}

fn chain(length: u64) -> Result<(Book, FiledRevisionContent), Box<dyn std::error::Error>> {
    let mut book = Book {
        ready: true,
        ..Book::default()
    };
    let mut parent = None;
    let mut last = None;
    for number in 1..=length {
        let revision = revision(number)?;
        let filed = FiledRevisionId::new(number)?;
        let position = FilingPosition {
            charter: CharterId::new(u32::try_from(number)?)?,
            line: LineId::new(number)?,
            sequence: FilingSequence::new(1)?,
        };
        let value = FiledRevisionContent::capture(
            filed,
            position,
            &revision,
            time(i64::try_from(number)?)?,
            parent,
        )?;
        book.lines.insert(position.line, position.charter);
        book.revisions.insert(revision.reference().key, revision);
        book.contents.insert(filed, value.clone());
        parent = Some(ParentContent {
            filed,
            revision: value.revision(),
        });
        last = Some(value);
    }
    Ok((book, last.ok_or("empty fixture chain")?))
}

fn changed(
    value: &FiledRevisionContent,
    field: &str,
    replacement: Value,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut document = serde_json::to_value(value)?;
    document
        .as_object_mut()
        .ok_or("content object")?
        .insert(field.into(), replacement);
    Ok(serde_json::to_vec(&document)?)
}

#[test]
fn canonical_content_roundtrips_without_source_or_authority() -> TestResult {
    let (book, content) = chain(4)?;
    content.check_links(&book)?;
    let bytes = content.encode()?;
    assert!(bytes.len() < MAX_CONTENT_BYTES);
    assert_eq!(
        FiledRevisionContent::decode(&content.key(), &bytes)?,
        content
    );
    assert_eq!(content.encode()?, bytes);
    let value: Value = serde_json::from_slice(&bytes)?;
    let keys: BTreeSet<_> = value
        .as_object()
        .ok_or("object")?
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from([
            "filed", "filed_at", "format", "parent", "position", "revision",
        ])
    );
    let text = String::from_utf8(bytes)?;
    assert!(!text.contains("private executable"));
    assert!(!text.contains("private notes"));
    assert!(!text.contains("rating"));
    assert!(!text.contains("holder"));
    assert!(!text.contains("certified"));
    Ok(())
}

#[test]
fn maximum_scalar_widths_fit_without_rounding_or_numeric_aliases() -> TestResult {
    let revision = revision(u64::MAX)?;
    let content = FiledRevisionContent::capture(
        FiledRevisionId::new(u64::MAX)?,
        FilingPosition {
            charter: CharterId::new(u32::MAX)?,
            line: LineId::new(u64::MAX)?,
            sequence: FilingSequence::new(u64::MAX)?,
        },
        &revision,
        time(i64::MIN)?,
        Some(ParentContent {
            filed: FiledRevisionId::new(u64::MAX - 1)?,
            revision: RevisionIdentity::capture(&revision)?,
        }),
    )?;
    assert_eq!(content.key(), "filed/ffffffffffffffff");
    let bytes = content.encode()?;
    assert!(bytes.len() < MAX_CONTENT_BYTES);
    assert_eq!(
        FiledRevisionContent::decode(&content.key(), &bytes)?,
        content
    );
    for bad in [
        json!(0),
        json!(1),
        json!("0"),
        json!("01"),
        json!("+1"),
        json!("1.0"),
        json!("18446744073709551616"),
    ] {
        assert!(serde_json::from_value::<FilingSequence>(bad).is_err());
    }
    Ok(())
}

#[test]
fn strict_version_key_unknown_missing_duplicate_and_byte_bounds() -> TestResult {
    let (_, content) = chain(1)?;
    for version in [0, 2] {
        assert_eq!(
            FiledRevisionContent::decode(
                &content.key(),
                &changed(&content, "format", json!(version))?
            ),
            Err(ContentError::Version)
        );
    }
    for key in [
        "filed/1",
        "filed/0000000000000002",
        "line/0000000000000001",
        "FILED/0000000000000001",
    ] {
        assert_eq!(
            FiledRevisionContent::decode(key, &content.encode()?),
            Err(ContentError::Key)
        );
    }
    assert_eq!(
        FiledRevisionContent::decode(&content.key(), &changed(&content, "extra", json!(true))?),
        Err(ContentError::Encoding)
    );
    for field in ["parent", "revision", "position", "filed_at"] {
        let mut value = serde_json::to_value(&content)?;
        value.as_object_mut().ok_or("object")?.remove(field);
        assert!(
            FiledRevisionContent::decode(&content.key(), &serde_json::to_vec(&value)?).is_err()
        );
    }
    let text = String::from_utf8(content.encode()?)?;
    let duplicate = text.replacen("{", "{\"format\":1,", 1);
    assert_eq!(
        FiledRevisionContent::decode(&content.key(), duplicate.as_bytes()),
        Err(ContentError::Encoding)
    );
    assert_eq!(
        FiledRevisionContent::decode(&content.key(), &vec![b' '; MAX_CONTENT_BYTES + 1]),
        Err(ContentError::Encoding)
    );
    Ok(())
}

#[test]
fn nested_unknown_and_missing_identity_fields_are_not_ignored() -> TestResult {
    let (_, content) = chain(2)?;
    for (field, required) in [
        ("position", "line"),
        ("parent", "revision"),
        ("revision", "source"),
    ] {
        let mut value = serde_json::to_value(&content)?;
        value
            .get_mut(field)
            .and_then(Value::as_object_mut)
            .ok_or("nested object")?
            .insert("extra".into(), json!(1));
        assert!(
            FiledRevisionContent::decode(&content.key(), &serde_json::to_vec(&value)?).is_err()
        );
        let mut value = serde_json::to_value(&content)?;
        value
            .get_mut(field)
            .and_then(Value::as_object_mut)
            .ok_or("nested object")?
            .remove(required);
        assert!(
            FiledRevisionContent::decode(&content.key(), &serde_json::to_vec(&value)?).is_err()
        );
    }
    Ok(())
}

#[test]
fn unavailable_is_distinct_from_available_empty_and_valid_parentless_content() -> TestResult {
    let (mut book, content) = chain(1)?;
    content.check_links(&book)?;
    book.ready = false;
    assert_eq!(content.check_links(&book), Err(ContentError::Unavailable));
    let empty = Book {
        ready: true,
        ..Book::default()
    };
    assert_eq!(content.check_links(&empty), Err(ContentError::Missing));
    Ok(())
}

#[test]
fn line_charter_saved_revision_and_source_must_match_exactly() -> TestResult {
    let (mut book, content) = chain(1)?;
    book.lines
        .insert(content.position.line, CharterId::new(99)?);
    assert_eq!(content.check_links(&book), Err(ContentError::Mismatch));
    book.lines
        .insert(content.position.line, content.position.charter);
    let key = content.revision.revision().key;
    let saved = book.revisions.remove(&key).ok_or("saved revision")?;
    assert_eq!(content.check_links(&book), Err(ContentError::Missing));
    let mut changed = saved.clone();
    changed.source = crate::source_digest("different source");
    book.revisions.insert(key, changed);
    assert_eq!(content.check_links(&book), Err(ContentError::Mismatch));
    let mut changed = saved;
    changed.reference.digest = crate::source_digest("different revision");
    book.revisions.insert(key, changed);
    assert_eq!(content.check_links(&book), Err(ContentError::Mismatch));
    Ok(())
}

#[test]
fn actual_parent_chain_rejects_missing_swapped_or_future_parent() -> TestResult {
    let (mut book, content) = chain(2)?;
    let parent = content.parent.ok_or("parent")?;
    let original = book.contents.remove(&parent.filed).ok_or("parent record")?;
    assert_eq!(content.check_links(&book), Err(ContentError::Missing));
    let mut wrong = original.clone();
    wrong.filed = FiledRevisionId::new(88)?;
    book.contents.insert(parent.filed, wrong);
    assert_eq!(content.check_links(&book), Err(ContentError::Mismatch));
    let mut wrong = original.clone();
    wrong.revision = RevisionIdentity::capture(&revision(99)?)?;
    book.contents.insert(parent.filed, wrong);
    assert_eq!(content.check_links(&book), Err(ContentError::Mismatch));
    let mut wrong = original.clone();
    wrong.filed_at = time(99)?;
    book.contents.insert(parent.filed, wrong);
    assert_eq!(content.check_links(&book), Err(ContentError::Mismatch));
    book.contents.insert(parent.filed, original);
    content.check_links(&book)?;
    Ok(())
}

#[test]
fn parent_depth_and_actual_cycles_are_bounded_without_recursive_decode() -> TestResult {
    let (book, content) = chain(4)?;
    content.check_links(&book)?;
    let (book, too_deep) = chain(5)?;
    assert_eq!(too_deep.check_links(&book), Err(ContentError::Ancestry));
    let (mut book, mut content) = chain(2)?;
    content.filed_at = time(1)?;
    let parent = content.parent.ok_or("parent")?;
    let first = book.contents.get_mut(&parent.filed).ok_or("first")?;
    first.parent = Some(ParentContent {
        filed: content.filed,
        revision: content.revision,
    });
    book.contents.insert(content.filed, content.clone());
    assert_eq!(content.check_links(&book), Err(ContentError::Ancestry));
    let bad = changed(
        &content,
        "parent",
        serde_json::to_value(ParentContent {
            filed: content.filed,
            revision: content.revision,
        })?,
    )?;
    assert_eq!(
        FiledRevisionContent::decode(&content.key(), &bad),
        Err(ContentError::Ancestry)
    );
    Ok(())
}

#[test]
fn an_ancestor_saved_source_or_line_mismatch_is_not_hidden_by_a_valid_child() -> TestResult {
    let (mut book, content) = chain(4)?;
    let ancestor = FiledRevisionId::new(1)?;
    let record = book.contents.get(&ancestor).ok_or("ancestor")?;
    book.revisions.remove(&record.revision.revision().key);
    assert_eq!(content.check_links(&book), Err(ContentError::Missing));
    let (mut book, content) = chain(4)?;
    book.lines.remove(&LineId::new(1)?);
    assert_eq!(content.check_links(&book), Err(ContentError::Missing));
    Ok(())
}
