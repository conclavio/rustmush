//! The `.sheet` keyframe walked generically into rows (interface plan I2,
//! §8 q12) — a port of the web client's `sheet.ts` rules: dots and narrow
//! numbers as pip rows, wide numbers as a value with a bar, tracks as boxes
//! with the deepest level named, maps alphabetically, lists joined, enums
//! capitalised. Everything is read defensively: a malformed field degrades
//! to a placeholder, never a panic (invariant T6).
//!
//! The payload is `{ target, schema: { ruleset, version, sections: [ { id,
//! label, layout, fields: [ { id, label, ty: { kind, … }, staff_only? } ] }
//! ] }, approved: { state, approved }, data: { <field id>: value } }`. A
//! field absent from `data` is not rendered, which is how a staff-only
//! field a player may not see stays invisible without this walker ever
//! checking `staff_only`.

use serde_json::Value;

/// A `Number` field renders as pips when its span is this small or
/// smaller (Willpower 1..10, Generation 4..15); wider pools show a value
/// and a bar.
pub const PIP_MAX_SPAN: i64 = 20;
/// The widest bar drawn for a wide-range number.
const BAR_W: usize = 10;

/// One rendered sheet row: a label column and a value column, or a
/// section header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SheetRow {
    /// A section title.
    Header(String),
    /// `label` and its rendered value.
    Field {
        /// The field label, indented for map entries.
        label: String,
        /// The rendered value.
        value: String,
    },
    /// A blank spacer.
    Blank,
}

/// Glyphs the walker uses; ASCII when the terminal has no UTF-8.
#[derive(Clone, Copy, Debug)]
pub struct Glyphs {
    /// A filled pip.
    pub pip_on: char,
    /// An empty pip.
    pub pip_off: char,
    /// A filled bar cell.
    pub bar_on: char,
    /// An empty bar cell.
    pub bar_off: char,
    /// An empty track box.
    pub box_empty: char,
}

impl Glyphs {
    /// Glyphs for a terminal of the given UTF-8 capability.
    #[must_use]
    pub fn for_utf8(utf8: bool) -> Glyphs {
        if utf8 {
            Glyphs {
                pip_on: '●',
                pip_off: '○',
                bar_on: '▰',
                bar_off: '▱',
                box_empty: '□',
            }
        } else {
            Glyphs {
                pip_on: '*',
                pip_off: '.',
                bar_on: '#',
                bar_off: '-',
                box_empty: '_',
            }
        }
    }
}

/// Walk a `.sheet` keyframe into rows. `None` when the payload has no
/// usable schema.
#[must_use]
pub fn sheet_rows(payload: &Value, glyphs: Glyphs) -> Option<Vec<SheetRow>> {
    let schema = payload.get("schema")?;
    let sections = schema.get("sections")?.as_array()?;
    let data = payload.get("data").and_then(Value::as_object);
    let mut rows = Vec::new();
    let approval = payload.get("approved");
    let state = approval
        .and_then(|a| a.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let ruleset = schema
        .get("ruleset")
        .and_then(Value::as_str)
        .unwrap_or("sheet");
    let target = payload.get("target").and_then(Value::as_str).unwrap_or("");
    rows.push(SheetRow::Field {
        label: format!("{ruleset} sheet"),
        value: format!("{target}  {state}"),
    });
    for section in sections {
        let Some(label) = section.get("label").and_then(Value::as_str) else {
            continue;
        };
        let Some(fields) = section.get("fields").and_then(Value::as_array) else {
            continue;
        };
        let visible: Vec<&Value> = fields
            .iter()
            .filter(|f| {
                f.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| data.is_some_and(|d| d.contains_key(id)))
            })
            .collect();
        if visible.is_empty() {
            continue;
        }
        rows.push(SheetRow::Blank);
        rows.push(SheetRow::Header(label.to_string()));
        for f in visible {
            let id = f.get("id").and_then(Value::as_str).unwrap_or("");
            let label = f.get("label").and_then(Value::as_str).unwrap_or(id);
            let ty = f.get("ty").cloned().unwrap_or(Value::Null);
            let value = data.and_then(|d| d.get(id)).cloned().unwrap_or(Value::Null);
            render_field(&mut rows, label, &ty, &value, glyphs, 0);
        }
    }
    Some(rows)
}

fn render_field(
    rows: &mut Vec<SheetRow>,
    label: &str,
    ty: &Value,
    value: &Value,
    glyphs: Glyphs,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let kind = ty.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "map" => {
            let inner = ty.get("value").cloned().unwrap_or(Value::Null);
            let Some(obj) = value.as_object() else {
                rows.push(SheetRow::Field {
                    label: format!("{indent}{label}"),
                    value: "(none)".to_string(),
                });
                return;
            };
            rows.push(SheetRow::Field {
                label: format!("{indent}{label}"),
                value: if obj.is_empty() {
                    "(none)".to_string()
                } else {
                    String::new()
                },
            });
            let mut entries: Vec<(&String, &Value)> = obj.iter().collect();
            entries.sort_by_key(|a| a.0.to_lowercase());
            if depth >= 2 {
                return;
            }
            for (k, v) in entries {
                render_field(rows, k, &inner, v, glyphs, depth + 1);
            }
        }
        _ => rows.push(SheetRow::Field {
            label: format!("{indent}{label}"),
            value: scalar_text(kind, ty, value, glyphs),
        }),
    }
}

/// A scalar field's display text by its declared kind.
fn scalar_text(kind: &str, ty: &Value, value: &Value, glyphs: Glyphs) -> String {
    match kind {
        "dots" => {
            let max = ty.get("max").and_then(Value::as_i64).unwrap_or(5);
            pip_row(value.as_i64().unwrap_or(0), max, glyphs)
        }
        "number" => {
            let min = ty.get("min").and_then(Value::as_i64).unwrap_or(0);
            let max = ty.get("max").and_then(Value::as_i64).unwrap_or(0);
            let v = value.as_i64().unwrap_or(0);
            if (max - min.min(0)).max(0) <= PIP_MAX_SPAN && max > 0 {
                pip_row(v, max, glyphs)
            } else {
                format!("{v:>3}/{max}  {}", bar(v, min, max, glyphs))
            }
        }
        "text" => value.as_str().unwrap_or("").to_string(),
        "enum" => capitalize(value.as_str().unwrap_or("")),
        "track" => {
            let cells = value.as_array().map_or_else(Vec::new, |a| {
                a.iter()
                    .map(|c| c.as_str().map(str::to_string))
                    .collect::<Vec<Option<String>>>()
            });
            let mut s = String::new();
            for c in &cells {
                s.push(c.as_deref().map_or(glyphs.box_empty, track_mark));
            }
            let filled = cells.iter().filter(|c| c.is_some()).count();
            let levels = ty.get("levels").and_then(Value::as_array);
            if filled > 0
                && let Some(levels) = levels
                && !levels.is_empty()
            {
                let lvl = &levels[(filled - 1).min(levels.len() - 1)];
                let name = lvl.get("label").and_then(Value::as_str).unwrap_or("");
                let penalty = lvl.get("penalty").and_then(Value::as_i64).unwrap_or(0);
                s.push_str(&format!("  {name} {penalty}"));
            }
            s
        }
        "list" => value.as_array().map_or_else(
            || "(none)".to_string(),
            |a| {
                let items: Vec<String> = a
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                if items.is_empty() {
                    "(none)".to_string()
                } else {
                    items.join(", ")
                }
            },
        ),
        _ => match value {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        },
    }
}

/// `value` filled pips out of `max` (a dot row's length is its ceiling).
fn pip_row(value: i64, max: i64, glyphs: Glyphs) -> String {
    let n = usize::try_from(max.max(0)).unwrap_or(0).min(20);
    let v = usize::try_from(value.max(0)).unwrap_or(0).min(n);
    let mut s = String::with_capacity(n);
    for i in 0..n {
        s.push(if i < v { glyphs.pip_on } else { glyphs.pip_off });
    }
    s
}

fn bar(value: i64, min: i64, max: i64, glyphs: Glyphs) -> String {
    let span = max - min;
    let filled = if span <= 0 {
        0
    } else {
        let v = value.clamp(min, max) - min;
        usize::try_from((v * BAR_W as i64 + span / 2) / span).unwrap_or(0)
    };
    let mut s = String::new();
    for i in 0..BAR_W {
        s.push(if i < filled {
            glyphs.bar_on
        } else {
            glyphs.bar_off
        });
    }
    s
}

/// A track mark's classic glyph: bashing `/`, lethal `X`, aggravated `*`;
/// anything else its first letter, uppercased.
fn track_mark(mark: &str) -> char {
    match mark {
        "bashing" => '/',
        "lethal" => 'X',
        "aggravated" => '*',
        other => other.chars().next().map_or('?', |c| c.to_ascii_uppercase()),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Value {
        serde_json::json!({
            "target": "#5",
            "schema": {
                "ruleset": "wod",
                "version": 1,
                "sections": [
                    { "id": "attrs", "label": "Attributes", "layout": { "grid": { "rows": 3, "cols": 3 } },
                      "fields": [
                        { "id": "strength", "label": "Strength", "ty": { "kind": "dots", "min": 1, "max": 5 }, "default": 1 },
                        { "id": "secret", "label": "Secret", "ty": { "kind": "number", "min": 0, "max": 9 }, "default": 0, "staff_only": true }
                      ] },
                    { "id": "pools", "label": "Pools", "layout": "list",
                      "fields": [
                        { "id": "willpower", "label": "Willpower", "ty": { "kind": "number", "min": 1, "max": 10 }, "default": 1 },
                        { "id": "bloodpool", "label": "Blood Pool", "ty": { "kind": "number", "min": 0, "max": 50 }, "default": 0 },
                        { "id": "template", "label": "Template", "ty": { "kind": "enum", "options": ["mortal", "vampire"] }, "default": "mortal" },
                        { "id": "health", "label": "Health", "ty": { "kind": "track", "size": { "fixed": 7 },
                          "levels": [ { "id": "bruised", "label": "Bruised", "penalty": 0 }, { "id": "hurt", "label": "Hurt", "penalty": -1 } ] }, "default": [] },
                        { "id": "disciplines", "label": "Disciplines", "ty": { "kind": "map", "value": { "kind": "dots", "min": 0, "max": 5 } }, "default": {} },
                        { "id": "languages", "label": "Languages", "ty": { "kind": "list", "of": { "kind": "text", "max_len": 20 } }, "default": [] }
                      ] },
                    { "id": "empty", "label": "Nothing here", "layout": "list", "fields": [] }
                ]
            },
            "approved": { "state": "approved", "approved": true },
            "data": {
                "strength": 3,
                "willpower": 6,
                "bloodpool": 25,
                "template": "vampire",
                "health": ["bashing", "lethal", null, null, null, null, null],
                "disciplines": { "Potence": 2, "Celerity": 1 },
                "languages": ["French", "Latin"]
            }
        })
    }

    fn text(rows: &[SheetRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                SheetRow::Header(h) => format!("# {h}"),
                SheetRow::Field { label, value } => format!("{label}: {value}"),
                SheetRow::Blank => String::new(),
            })
            .collect()
    }

    #[test]
    fn walks_every_kind_and_skips_absent_fields() {
        let rows = sheet_rows(&payload(), Glyphs::for_utf8(false)).unwrap();
        let t = text(&rows);
        assert_eq!(t[0], "wod sheet: #5  approved");
        assert!(t.contains(&"# Attributes".to_string()));
        assert!(t.contains(&"Strength: ***..".to_string()));
        assert!(
            !t.iter().any(|l| l.starts_with("Secret")),
            "absent staff field"
        );
        assert!(t.contains(&"Willpower: ******....".to_string()));
        assert!(
            t.iter()
                .any(|l| l.starts_with("Blood Pool:  25/50  #####-----")),
            "{t:?}"
        );
        assert!(t.contains(&"Template: Vampire".to_string()));
        assert!(t.contains(&"Health: /X_____  Hurt -1".to_string()));
        assert!(t.contains(&"Disciplines: ".to_string()));
        assert!(t.contains(&"  Celerity: *....".to_string()));
        assert!(t.contains(&"  Potence: **...".to_string()));
        assert!(t.contains(&"Languages: French, Latin".to_string()));
        assert!(
            !t.iter().any(|l| l.contains("Nothing here")),
            "empty sections vanish"
        );
    }

    #[test]
    fn malformed_payloads_degrade_instead_of_panicking() {
        assert!(sheet_rows(&serde_json::json!({}), Glyphs::for_utf8(true)).is_none());
        assert!(sheet_rows(&serde_json::json!({ "schema": 5 }), Glyphs::for_utf8(true)).is_none());
        let odd = serde_json::json!({
            "schema": { "sections": [ { "label": "X", "fields": [
                { "id": "a", "label": "A", "ty": { "kind": "dots", "max": "no" } },
                { "id": "b", "label": "B", "ty": { "kind": "track" } },
                { "id": "c", "label": "C", "ty": { "kind": "map", "value": { "kind": "dots" } } },
                { "id": "d", "label": "D" }
            ] } ] },
            "data": { "a": "three", "b": "not a track", "c": [1, 2], "d": { "x": 1 } }
        });
        let rows = sheet_rows(&odd, Glyphs::for_utf8(true)).unwrap();
        let t = text(&rows);
        assert!(t.iter().any(|l| l.starts_with("A: ")));
        assert!(t.contains(&"C: (none)".to_string()));
        assert!(t.iter().any(|l| l.starts_with("D: ")));
    }

    #[test]
    fn pips_and_bars_are_bounded() {
        let g = Glyphs::for_utf8(false);
        assert_eq!(pip_row(7, 5, g), "*****");
        assert_eq!(pip_row(-2, 3, g), "...");
        assert_eq!(pip_row(3, 100, g).len(), 20);
        assert_eq!(bar(50, 0, 100, g), "#####-----");
        assert_eq!(bar(5, 0, 0, g), "----------");
    }
}
