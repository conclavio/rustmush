//! Character-owned authoring services shared by line, web and SSH entry points.
//!
//! Authenticated adapters supply the current character, never an owner from a
//! request body. Every source-bearing operation resolves through `owned_copy`.
//! Working edits do not compile; immutable saves use an opaque worker receipt.

pub(crate) mod author_rules;
pub(crate) mod commands;
pub(crate) mod executor;
pub(crate) mod structured;

use mush_core::ObjId;
use mush_roll::{
    EditBase, Envelope, PreparedChange, RevisionKey, RollError, RollStore, SourceAdapter,
    WorkingCopy,
};
use serde_json::Value;
use smol_str::SmolStr;

use crate::arena::core::CoreClass;
use crate::arena::schema::{CardValue, ValidatedEnvelope, Widget};

/// Output is independently bounded, even though stored input is already bounded.
pub(crate) const MAX_OUTPUT_LINES: usize = 200;
pub(crate) const MAX_OUTPUT_BYTES: usize = 40 * 1024;

pub(crate) fn invalid(field: &str, message: impl Into<String>) -> RollError {
    RollError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

/// An owned reference is a library label, optionally followed by a local
/// revision number or named version. Object ids and cross-character paths have
/// no interpretation. Immutable digest lookup is explicit `@sha:<hex>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MindRef {
    name: String,
    revision: Option<String>,
}

impl MindRef {
    pub(crate) fn parse(text: &str) -> Result<Self, RollError> {
        let (name, revision) = text
            .split_once('@')
            .map_or((text, None), |(name, rev)| (name, Some(rev.to_owned())));
        let name = mush_roll::canonical_name(name)?;
        if revision.as_ref().is_some_and(|rev| {
            rev.is_empty()
                || rev.len() > 68
                || rev.contains('@')
                || rev.chars().any(char::is_control)
        }) {
            return Err(invalid(
                "revision",
                "expected a local revision number, version name or sha:<digest>",
            ));
        }
        Ok(Self { name, revision })
    }
}

/// One source authorization predicate. It has no staff bypass and no parameter
/// for another character. Read, copy, diff, check and both remote adapters use it.
pub(crate) fn owned_copy<S: SourceAdapter>(
    roll: &RollStore,
    sources: &S,
    actor: ObjId,
    reference: &MindRef,
) -> Result<WorkingCopy, RollError> {
    let mind = roll.mind(actor, &reference.name)?;
    let Some(revision) = &reference.revision else {
        return Ok(mind.draft().clone());
    };
    let key = if let Some(digest) = revision.strip_prefix("sha:") {
        roll.resolve_digest(actor, &reference.name, digest)?.key
    } else {
        let number = revision
            .parse::<u64>()
            .ok()
            .filter(|number| *number > 0)
            .or_else(|| mind.named().get(&revision.to_ascii_lowercase()).copied())
            .ok_or(RollError::Missing)?;
        RevisionKey {
            character: actor,
            mind: mind.id(),
            number,
        }
    };
    let revision = roll.revision(actor, key)?;
    let source = sources.read_source(revision.source())?;
    revision.verify_source(sources)?;
    Ok(WorkingCopy {
        source: source.to_owned(),
        envelope: revision.envelope().clone(),
    })
}

pub(crate) fn create<S: SourceAdapter>(
    roll: &RollStore,
    sources: &S,
    actor: ObjId,
    name: &str,
    from: Option<&str>,
) -> Result<PreparedChange, RollError> {
    if name.eq_ignore_ascii_case("Standard") {
        return Err(invalid("name", "Standard is the reserved Registry mind"));
    }
    let draft = match from {
        None => WorkingCopy {
            source: "// A unit mind receives only its own published picture.\nfn think(view, card) {\n}\n".into(),
            envelope: Envelope { schema: serde_json::json!({"sections": []}), presets: serde_json::json!({}), profiles: serde_json::json!({"Default": {}}), rules: serde_json::json!({}), declared_class: "I".into(), mind_abi: crate::minds::MIND_ABI.into() },
        },
        Some(name) if name.eq_ignore_ascii_case("Standard") => crate::minds::starter::standard_draft()?,
        Some(reference) => owned_copy(roll, sources, actor, &MindRef::parse(reference)?)?,
    };
    roll.prepare_create(actor, name, draft)
}

/// Shared owner transaction for line and structured lifecycle requests. No
/// library mutation can bypass the exact working-copy token or character ACL.
pub(super) fn lifecycle(
    roll: &RollStore,
    actor: ObjId,
    name: &str,
    base: EditBase,
    edit: &mush_proto::MindLifecycle,
) -> Result<PreparedChange, RollError> {
    match edit {
        mush_proto::MindLifecycle::Rename { new_name } => {
            if new_name.trim().eq_ignore_ascii_case("Standard") {
                return Err(invalid("name", "Standard is the reserved Registry mind"));
            }
            roll.prepare_rename(actor, name, base, new_name)
        }
        mush_proto::MindLifecycle::Remove => roll.prepare_remove(actor, name, base),
        mush_proto::MindLifecycle::VersionRename { old, new } => {
            roll.prepare_rename_version(actor, name, base, old, new)
        }
        mush_proto::MindLifecycle::VersionForget { version } => {
            roll.prepare_forget_version(actor, name, base, version)
        }
    }
}

/// Exact text editing. Line indices are one-based; line endings and final
/// newline survive edits outside their range. CRLF normalization belongs to save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TextEdit {
    Replace { line: usize, text: String },
    Insert { line: usize, text: String },
    Delete { first: usize, last: usize },
    ReplaceAll(String),
}

pub(crate) fn edit_text(
    roll: &RollStore,
    actor: ObjId,
    name: &str,
    base: EditBase,
    edit: TextEdit,
) -> Result<PreparedChange, RollError> {
    let mind = roll.mind(actor, name)?;
    if mind.base() != base {
        return Err(RollError::StaleEdit);
    }
    let mut draft = mind.draft().clone();
    draft.source = apply_text(&draft.source, edit)?;
    roll.prepare_edit(actor, name, base, draft)
}

fn apply_text(source: &str, edit: TextEdit) -> Result<String, RollError> {
    let mut lines = source
        .split_inclusive('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let result = match edit {
        TextEdit::ReplaceAll(text) => text,
        TextEdit::Replace { line, text } => {
            single_line(&text)?;
            let slot = line
                .checked_sub(1)
                .and_then(|index| lines.get_mut(index))
                .ok_or_else(|| invalid("line", "outside the working source"))?;
            let ending = if slot.ends_with("\r\n") {
                "\r\n"
            } else if slot.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            *slot = format!("{text}{ending}");
            lines.concat()
        }
        TextEdit::Insert { line, text } => {
            single_line(&text)?;
            let index = line
                .checked_sub(1)
                .filter(|index| *index <= lines.len())
                .ok_or_else(|| invalid("line", "outside the working source"))?;
            if index == lines.len()
                && let Some(last) = lines.last_mut().filter(|last| !last.ends_with('\n'))
            {
                last.push('\n');
            }
            lines.insert(index, format!("{text}\n"));
            lines.concat()
        }
        TextEdit::Delete { first, last } => {
            if first == 0 || last < first || last > lines.len() {
                return Err(invalid("line", "invalid source range"));
            }
            lines.drain(first - 1..last);
            lines.concat()
        }
    };
    if result.len() > mush_roll::MAX_SOURCE_BYTES || result.contains('\0') {
        return Err(invalid("source", "maximum 32 KiB; NUL is forbidden"));
    }
    Ok(result)
}

fn single_line(text: &str) -> Result<(), RollError> {
    if text.contains(['\r', '\n', '\0']) {
        Err(invalid("line", "line edits cannot contain newline or NUL"))
    } else {
        Ok(())
    }
}

pub(crate) fn decoded(envelope: &Envelope) -> Result<ValidatedEnvelope, RollError> {
    envelope.validate()?;
    let class = CoreClass::parse(&envelope.declared_class)
        .ok_or_else(|| invalid("class", "expected I, II or III"))?;
    crate::arena::schema::validate_envelope(
        &envelope.schema,
        &envelope.presets,
        &envelope.profiles,
        &envelope.rules,
        class,
    )
    .map_err(|error| invalid("card", error.to_string()))
}

fn write_metadata(envelope: &mut Envelope, card: &ValidatedEnvelope) -> Result<(), RollError> {
    envelope.schema = value(&card.schema)?;
    envelope.profiles = value(&card.profiles)?;
    envelope.presets = value(&card.presets)?;
    envelope.rules = value(&card.rules)?;
    decoded(envelope)?;
    Ok(())
}

fn value<T: serde::Serialize>(value: &T) -> Result<Value, RollError> {
    serde_json::to_value(value).map_err(|_| invalid("card", "cannot encode metadata"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardEdit {
    Set {
        profile: String,
        field: String,
        value: String,
    },
    PresetSave {
        profile: String,
        preset: String,
    },
    PresetApply {
        profile: String,
        preset: String,
    },
    PresetDelete {
        preset: String,
    },
    ProfileAdd {
        name: String,
        from: Option<String>,
    },
    ProfileDelete {
        name: String,
    },
    ProfileRename {
        old: String,
        new: String,
    },
    Rule {
        profile: String,
        slot: usize,
        text: String,
    },
}

/// All metadata editors decode and re-encode the same strict envelope. They do
/// not alter source or execute scripts, and invalid edits leave the draft intact.
pub(crate) fn edit_card(
    roll: &RollStore,
    actor: ObjId,
    name: &str,
    base: EditBase,
    edit: CardEdit,
    floor_permille: u16,
) -> Result<PreparedChange, RollError> {
    let mind = roll.mind(actor, name)?;
    if mind.base() != base {
        return Err(RollError::StaleEdit);
    }
    let mut draft = mind.draft().clone();
    let mut card = decoded(&draft.envelope)?;
    match edit {
        CardEdit::Set {
            profile,
            field,
            value,
        } => {
            let definition = card
                .schema
                .sections
                .iter()
                .flat_map(|section| &section.fields)
                .find(|definition| definition.id == field)
                .ok_or_else(|| invalid("field", "unknown card field"))?;
            let parsed = parse_value(&definition.widget, &value)?;
            definition
                .widget
                .validate_value(&parsed, &field)
                .map_err(|error| invalid("value", error.to_string()))?;
            let values = card
                .profiles
                .get_mut(profile.as_str())
                .ok_or_else(|| invalid("profile", "unknown profile"))?;
            values.insert(field.into(), parsed);
        }
        CardEdit::PresetSave { profile, preset } => {
            let values = card
                .profiles
                .get(profile.as_str())
                .ok_or_else(|| invalid("profile", "unknown profile"))?
                .clone();
            card.presets.insert(preset.into(), values);
        }
        CardEdit::PresetApply { profile, preset } => {
            let values = card
                .presets
                .get(preset.as_str())
                .ok_or_else(|| invalid("preset", "unknown preset"))?
                .clone();
            *card
                .profiles
                .get_mut(profile.as_str())
                .ok_or_else(|| invalid("profile", "unknown profile"))? = values;
        }
        CardEdit::PresetDelete { preset } => {
            card.presets
                .remove(preset.as_str())
                .ok_or_else(|| invalid("preset", "unknown preset"))?;
        }
        CardEdit::ProfileAdd { name, from } => {
            if card.profiles.contains_key(name.as_str()) {
                return Err(invalid("profile", "name is in use"));
            }
            let copied_rules = from
                .as_ref()
                .and_then(|from| card.rules.get(from.as_str()))
                .cloned();
            let values = match from {
                Some(from) => card
                    .profiles
                    .get(from.as_str())
                    .ok_or_else(|| invalid("profile", "unknown source profile"))?
                    .clone(),
                None => card.schema.defaults(),
            };
            card.profiles.insert(name.clone().into(), values);
            if let Some(rules) = copied_rules {
                card.rules.insert(name.into(), rules);
            }
        }
        CardEdit::ProfileDelete { name } => {
            card.profiles
                .remove(name.as_str())
                .ok_or_else(|| invalid("profile", "unknown profile"))?;
            card.rules.remove(name.as_str());
        }
        CardEdit::ProfileRename { old, new } => {
            if card.profiles.contains_key(new.as_str()) {
                return Err(invalid("profile", "name is in use"));
            }
            let values = card
                .profiles
                .remove(old.as_str())
                .ok_or_else(|| invalid("profile", "unknown profile"))?;
            card.profiles.insert(new.clone().into(), values);
            if let Some(rules) = card.rules.remove(old.as_str()) {
                card.rules.insert(new.into(), rules);
            }
        }
        CardEdit::Rule {
            profile,
            slot,
            text,
        } => {
            if !card.profiles.contains_key(profile.as_str()) {
                return Err(invalid("profile", "unknown profile"));
            }
            let rules = card.rules.entry(profile.into()).or_default();
            if slot == 0 || slot > 6 || slot > rules.len() + 1 {
                return Err(invalid(
                    "rule slot",
                    "replace an existing row or append the next row",
                ));
            }
            if text.eq_ignore_ascii_case("off") {
                if slot > rules.len() {
                    return Err(invalid("rule slot", "no such row"));
                }
                rules.remove(slot - 1);
            } else {
                let rule = author_rules::parse(&text, floor_permille)?;
                if let Some(existing) = rules.get_mut(slot - 1) {
                    *existing = rule;
                } else {
                    rules.push(rule);
                }
            }
        }
    }
    write_metadata(&mut draft.envelope, &card)?;
    roll.prepare_edit(actor, name, base, draft)
}

fn parse_value(widget: &Widget, text: &str) -> Result<CardValue, RollError> {
    match widget {
        Widget::Slider { .. } | Widget::Bearing { .. } => text
            .parse::<i64>()
            .map(CardValue::Integer)
            .map_err(|_| invalid("value", "expected integer stored units")),
        Widget::Toggle { .. } => match text {
            "true" => Ok(CardValue::Boolean(true)),
            "false" => Ok(CardValue::Boolean(false)),
            _ => Err(invalid("value", "expected true or false")),
        },
        Widget::Choice { .. } => Ok(CardValue::Word(text.into())),
        Widget::FacetSet { .. } | Widget::ContactClassSet { .. } => {
            let words = if text.is_empty() {
                Vec::new()
            } else {
                text.split(',')
                    .map(|word| SmolStr::new(word.trim()))
                    .collect()
            };
            Ok(CardValue::Set(words))
        }
    }
}

/// Text for terminal display escapes controls without changing stored source.
pub(crate) fn display_text(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\t') {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}

pub(crate) fn source_lines(
    source: &str,
    first: usize,
    last: usize,
) -> Result<Vec<String>, RollError> {
    if first == 0 || last < first || last - first >= MAX_OUTPUT_LINES {
        return Err(invalid("range", "request at most 200 source lines"));
    }
    let lines = source
        .split('\n')
        .enumerate()
        .skip(first - 1)
        .take(last - first + 1)
        .map(|(index, line)| format!("{:>4} {}", index + 1, display_text(line)))
        .collect::<Vec<_>>();
    bounded_output(lines)
}

/// A linear, bounded prefix/suffix diff; no quadratic LCS over attacker text.
/// Source lines and the complete metadata change are both represented.
pub(crate) fn diff(left: &WorkingCopy, right: &WorkingCopy) -> Result<Vec<String>, RollError> {
    let left_lines = left.source.split('\n').collect::<Vec<_>>();
    let right_lines = right.source.split('\n').collect::<Vec<_>>();
    let prefix = left_lines
        .iter()
        .zip(&right_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = left_lines
        .iter()
        .skip(prefix)
        .rev()
        .zip(right_lines.iter().skip(prefix).rev())
        .take_while(|(left, right)| left == right)
        .count();
    let mut result = Vec::new();
    for line in left_lines
        .iter()
        .skip(prefix)
        .take(left_lines.len() - prefix - suffix)
    {
        result.push(format!("- {}", display_text(line)));
    }
    for line in right_lines
        .iter()
        .skip(prefix)
        .take(right_lines.len() - prefix - suffix)
    {
        result.push(format!("+ {}", display_text(line)));
    }
    if left.envelope != right.envelope {
        result.push(format!(
            "- card {}",
            display_text(
                &serde_json::to_string(&left.envelope)
                    .map_err(|_| invalid("diff", "cannot encode card"))?
            )
        ));
        result.push(format!(
            "+ card {}",
            display_text(
                &serde_json::to_string(&right.envelope)
                    .map_err(|_| invalid("diff", "cannot encode card"))?
            )
        ));
    }
    if result.is_empty() {
        result.push("No executable differences.".into());
    }
    bounded_output(result)
}

fn bounded_output(lines: Vec<String>) -> Result<Vec<String>, RollError> {
    if lines.len() > MAX_OUTPUT_LINES
        || lines.iter().map(String::len).sum::<usize>() > MAX_OUTPUT_BYTES
    {
        return Err(invalid(
            "output",
            "result is too large; request a smaller source range",
        ));
    }
    Ok(lines)
}

/// Render one complete bounded card response before emitting any line. Large
/// libraries remain readable profile by profile; no partial response is sent.
pub(crate) fn card_lines(
    card: &ValidatedEnvelope,
    profile: Option<&str>,
) -> Result<Vec<String>, RollError> {
    let mut lines = Vec::new();
    for (name, values) in &card.profiles {
        if profile.is_some_and(|profile| profile != name.as_str()) {
            continue;
        }
        lines.push(format!("Profile {name}:"));
        for (field, value) in values {
            lines.push(format!(
                "  {field}={}",
                serde_json::to_string(value).map_err(|_| invalid("card", "encoding failed"))?
            ));
        }
        for (index, rule) in card.rules.get(name).into_iter().flatten().enumerate() {
            lines.push(format!(
                "  rule {}: {}",
                index + 1,
                author_rules::words(rule)
            ));
        }
    }
    if lines.is_empty() {
        return Err(invalid("profile", "unknown profile"));
    }
    bounded_output(lines).map_err(|_| {
        invalid(
            "card",
            "response exceeds 200 lines or 40 KiB; request @mind/card name/profile",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn largest_four_profile_card_requires_a_bounded_profile_read() {
        let fields = (0..64).map(|index| serde_json::json!({"id":format!("field_{index}"),"label":"Field","widget":{"kind":"toggle","default":false,"help":"Toggle"}})).collect::<Vec<_>>();
        let values = (0..64)
            .map(|index| (format!("field_{index}"), Value::Bool(false)))
            .collect::<serde_json::Map<_, _>>();
        let profiles = (0..4)
            .map(|index| (format!("P{index}"), Value::Object(values.clone())))
            .collect::<serde_json::Map<_, _>>();
        let envelope = Envelope {
            schema: serde_json::json!({"sections":[{"id":"helm","label":"Helm","station":"helm","fields":fields}]}),
            presets: serde_json::json!({}),
            profiles: Value::Object(profiles),
            rules: serde_json::json!({}),
            declared_class: "III".into(),
            mind_abi: crate::minds::MIND_ABI.into(),
        };
        let card = decoded(&envelope).unwrap();
        assert!(card_lines(&card, None).is_err());
        assert_eq!(card_lines(&card, Some("P0")).unwrap().len(), 65);
    }
    #[test]
    fn line_edits_preserve_unedited_literal_bytes_and_endings() {
        let source = "let a = `one\r\ntwo`;\r\nold\nend";
        assert_eq!(
            apply_text(
                source,
                TextEdit::Replace {
                    line: 3,
                    text: "new".into()
                }
            )
            .unwrap(),
            "let a = `one\r\ntwo`;\r\nnew\nend"
        );
        assert_eq!(
            apply_text(
                "one",
                TextEdit::Insert {
                    line: 2,
                    text: "two".into()
                }
            )
            .unwrap(),
            "one\ntwo\n"
        );
        assert!(apply_text(source, TextEdit::Delete { first: 0, last: 1 }).is_err());
        assert!(
            apply_text(
                source,
                TextEdit::Replace {
                    line: 1,
                    text: "x\ny".into()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn controls_are_visible_and_never_terminal_commands() {
        assert_eq!(display_text("\u{1b}[2J\rtext"), "\\u{1b}[2J\\rtext");
        assert!(MindRef::parse("#99/secret").is_err());
        assert!(MindRef::parse("owned@@2").is_err());
    }
}
