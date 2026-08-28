//! The generic `+sheet` renderer: walk a [`SheetSchema`] + [`SheetDoc`] into
//! a classic terminal character sheet (section headers, dot pips like
//! `Strength: ***..`, damage tracks, maps and lists).
//!
//! Rendering is a pure `-> Vec<String>` function so it golden-tests cleanly;
//! the command layer wraps each line in an `OutputEvent::Sys`. A ruleset that
//! wants a fancier sheet overrides `+sheet` and ignores this — it is the
//! reusable default, not a mandate.

use crate::doc::SheetDoc;
use crate::schema::{FieldType, SheetSchema, SheetSection};

/// Total display width of the sheet (banner rules).
const WIDTH: usize = 72;
/// Column where dot pips / values start.
const VALUE_COL: usize = 22;

/// Render a full sheet. `show_staff` includes `staff_only` fields (pass the
/// result of a control/examine check); when false those fields are omitted.
#[must_use]
pub fn render_sheet(schema: &SheetSchema, doc: &SheetDoc, show_staff: bool) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(banner(&format!(
        "{} sheet [{}]",
        schema.ruleset.to_uppercase(),
        doc.approved.label()
    )));
    for section in &schema.sections {
        render_section(section, doc, show_staff, &mut lines);
    }
    lines.push(rule('='));
    lines
}

fn render_section(
    section: &SheetSection,
    doc: &SheetDoc,
    show_staff: bool,
    lines: &mut Vec<String>,
) {
    // Which fields are visible?
    let visible: Vec<_> = section
        .fields
        .iter()
        .filter(|f| show_staff || !f.staff_only)
        .collect();
    if visible.is_empty() {
        return;
    }
    lines.push(section_header(&section.label));
    for f in visible {
        let value = doc.data.get(f.id.as_str());
        match &f.ty {
            FieldType::Dots { max, .. } => {
                let dots = value.and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;
                lines.push(labelled(&f.label, &pips(dots, *max as usize)));
            }
            FieldType::Number { .. } => {
                let n = value.and_then(serde_json::Value::as_i64).unwrap_or(0);
                lines.push(labelled(&f.label, &n.to_string()));
            }
            FieldType::Text { .. } | FieldType::Enum { .. } => {
                let s = value.and_then(serde_json::Value::as_str).unwrap_or("");
                lines.push(labelled(&f.label, s));
            }
            FieldType::Track { levels, .. } => {
                let marks = doc.track(f.id.as_str()).unwrap_or_default();
                lines.push(labelled(&f.label, &render_track(&marks)));
                if let Some(pen) = deepest_penalty(&marks, levels) {
                    lines.push(labelled("  wound penalty", &format!("{pen:+}")));
                }
            }
            FieldType::List { .. } => {
                let items: Vec<String> = value
                    .and_then(serde_json::Value::as_array)
                    .map(|a| a.iter().map(render_scalar).collect())
                    .unwrap_or_default();
                lines.push(labelled(&f.label, &items.join(", ")));
            }
            FieldType::Map { .. } => {
                lines.push(format!("{}:", f.label));
                if let Some(obj) = value.and_then(serde_json::Value::as_object) {
                    for (k, v) in obj {
                        let rendered = match v.as_u64() {
                            // Small dot-like values render as pips.
                            Some(n) if n <= 10 => pips(n as usize, 5.max(n as usize)),
                            _ => render_scalar(v),
                        };
                        lines.push(labelled(&format!("  {k}"), &rendered));
                    }
                }
            }
        }
    }
}

/// A dot rating as filled/empty pips: `***..` for 3 of 5.
#[must_use]
pub fn pips(filled: usize, max: usize) -> String {
    let filled = filled.min(max);
    let mut s = String::with_capacity(max);
    s.extend(std::iter::repeat_n('*', filled));
    s.extend(std::iter::repeat_n('.', max - filled));
    s
}

/// A track row: `.` per empty slot, the mark's first uppercased letter per
/// filled slot (`B`/`L`/`A` for bashing/lethal/aggravated).
fn render_track(marks: &[Option<smol_str::SmolStr>]) -> String {
    let mut s = String::with_capacity(marks.len() + 2);
    s.push('[');
    for m in marks {
        match m {
            None => s.push('.'),
            Some(mark) => s.push(
                mark.chars()
                    .next()
                    .map(|c| c.to_ascii_uppercase())
                    .unwrap_or('X'),
            ),
        }
    }
    s.push(']');
    s
}

/// The wound penalty of the deepest filled level, or `None` if undamaged /
/// no levels defined.
fn deepest_penalty(
    marks: &[Option<smol_str::SmolStr>],
    levels: &[crate::schema::DamageLevelDef],
) -> Option<i8> {
    let filled = marks.iter().filter(|m| m.is_some()).count();
    if filled == 0 || levels.is_empty() {
        return None;
    }
    let idx = (filled - 1).min(levels.len() - 1);
    Some(levels[idx].penalty)
}

fn render_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn labelled(label: &str, value: &str) -> String {
    let mut s = String::new();
    s.push_str(label);
    s.push(':');
    let used = label.chars().count() + 1;
    if used < VALUE_COL {
        s.extend(std::iter::repeat_n(' ', VALUE_COL - used));
    } else {
        s.push(' ');
    }
    s.push_str(value);
    s
}

fn section_header(label: &str) -> String {
    let upper = label.to_uppercase();
    let dashes = WIDTH.saturating_sub(upper.chars().count() + 4);
    format!("-- {upper} {}", "-".repeat(dashes))
}

fn banner(title: &str) -> String {
    let title = title.to_uppercase();
    let pad = WIDTH.saturating_sub(title.chars().count() + 2);
    let left = pad / 2;
    let right = pad - left;
    format!("{} {title} {}", "=".repeat(left), "=".repeat(right))
}

fn rule(c: char) -> String {
    c.to_string().repeat(WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        DamageLevelDef, FieldDef, FieldType, SectionLayout, SheetSchemaBuilder, SheetSection,
        TrackSize,
    };
    use smol_str::SmolStr;

    fn schema() -> SheetSchema {
        SheetSchemaBuilder::new("toy", 1)
            .section(
                SheetSection::new("attributes", "Attributes", SectionLayout::grid(1, 2))
                    .field(FieldDef::dots("strength", "Strength", 1, 5))
                    .field(FieldDef::dots("dexterity", "Dexterity", 1, 5)),
            )
            .unwrap()
            .section(
                SheetSection::new("bio", "Bio", SectionLayout::List)
                    .field(FieldDef::text("concept", "Concept", 40))
                    .field(FieldDef::dots("secret", "Secret", 0, 5).staff_only()),
            )
            .unwrap()
            .section(
                SheetSection::new("backgrounds", "Backgrounds", SectionLayout::Map).field(
                    FieldDef::map(
                        "backgrounds",
                        "Backgrounds",
                        FieldType::Dots { min: 0, max: 5 },
                    ),
                ),
            )
            .unwrap()
            .section(
                SheetSection::new("health", "Health", SectionLayout::Tracks).field(
                    FieldDef::track(
                        "health",
                        "Health",
                        TrackSize::Fixed(7),
                        vec![
                            DamageLevelDef::new("bruised", "Bruised", 0),
                            DamageLevelDef::new("hurt", "Hurt", -1),
                            DamageLevelDef::new("injured", "Injured", -1),
                            DamageLevelDef::new("wounded", "Wounded", -2),
                        ],
                    ),
                ),
            )
            .unwrap()
            .build()
    }

    fn sample() -> SheetDoc {
        let s = schema();
        let mut doc = SheetDoc::new_default(&s);
        doc.set_number("strength", 3);
        doc.set_number("dexterity", 2);
        doc.set_text("concept", "brooding detective");
        doc.set_number("secret", 4);
        doc.map_set("backgrounds", "Resources", serde_json::json!(2));
        doc.set_track(
            "health",
            &[
                Some(SmolStr::new("bashing")),
                Some(SmolStr::new("lethal")),
                None,
                None,
                None,
                None,
                None,
            ],
        );
        doc
    }

    #[test]
    fn pips_render() {
        assert_eq!(pips(3, 5), "***..");
        assert_eq!(pips(0, 5), ".....");
        assert_eq!(pips(7, 5), "*****"); // clamped
    }

    /// Reproduce a rendered `label: value` line, so golden assertions never
    /// hinge on hand-counted padding.
    fn line(label: &str, value: &str) -> String {
        labelled(label, value)
    }

    #[test]
    fn golden_player_view_hides_staff_only() {
        let out = render_sheet(&schema(), &sample(), false);
        assert!(out.contains(&line("Strength", "***..")));
        assert!(out.contains(&line("Dexterity", "**...")));
        assert!(out.contains(&line("Concept", "brooding detective")));
        // staff-only "Secret" hidden from players.
        assert!(out.iter().all(|l| !l.contains("Secret")));
        // Backgrounds map entry rendered as pips (2 of 5).
        assert!(out.contains(&line("  Resources", "**...")));
        // Health track + wound penalty (2 filled → level index 1 = Hurt -1).
        assert!(out.contains(&line("Health", "[BL.....]")));
        assert!(out.contains(&line("  wound penalty", "-1")));
    }

    #[test]
    fn staff_view_shows_staff_only() {
        let out = render_sheet(&schema(), &sample(), true);
        assert!(out.contains(&line("Secret", "****.")));
    }

    #[test]
    fn banner_shows_approval() {
        let out = render_sheet(&schema(), &sample(), false);
        assert!(out[0].contains("TOY SHEET [UNAPPROVED]"));
    }
}
