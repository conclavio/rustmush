//! Pool-expression resolution: turn `strength+brawl+2` (or a raw `7`) into a
//! dice pool by resolving each trait name against the roller's sheet.
//!
//! A pool expression is a `+`/`-` chain of terms:
//!
//! - a **trait name** — fuzzy prefix-matched (case-insensitive) against the
//!   sheet's attributes, abilities, virtues, and background names, contributing
//!   its dots;
//! - a bare **integer** — a flat modifier;
//! - a **`name:value`** term — an explicitly-valued named bonus ("bloodbuff:2").
//!
//! A pool expression that is a single integer (`+roll 7`) needs no sheet.
//! Resolution is pure and returns the full per-term breakdown for the roll log.

use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{FieldType, SheetSchema};

/// One resolved contribution to a pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolTerm {
    /// Human label ("Strength", "Brawl", "+2", "bloodbuff").
    pub label: String,
    /// Signed dice contributed.
    pub value: i32,
}

/// A fully resolved pool: the term breakdown and the summed total (before
/// wound penalty / willpower, which the roll layer applies).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPool {
    /// Every term, in expression order.
    pub terms: Vec<PoolTerm>,
    /// The summed dice.
    pub total: i32,
    /// The trait ids that resolved to real ability/attribute fields, so the
    /// roll layer can look up specialties (`/spec` without a name).
    pub trait_fields: Vec<String>,
}

/// Normalize a name for matching: lowercase, keep only alphanumerics.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A candidate trait the resolver can match: `(field_id_or_none, label, dots)`.
struct Candidate {
    field: Option<String>,
    label: String,
    dots: i32,
}

/// Collect every matchable trait on the sheet: dot-typed attribute/ability/
/// virtue fields plus background map entries.
fn candidates(schema: &SheetSchema, doc: &SheetDoc) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (_, f) in schema.fields() {
        if matches!(f.ty, FieldType::Dots { .. }) {
            out.push(Candidate {
                field: Some(f.id.to_string()),
                label: f.label.clone(),
                dots: doc.number(&f.id).unwrap_or(0) as i32,
            });
        }
        // Background-style maps of dots contribute their named entries.
        if let FieldType::Map { value } = &f.ty
            && matches!(**value, FieldType::Dots { .. })
            && let Some(entries) = doc.map(&f.id)
        {
            for (k, v) in entries {
                out.push(Candidate {
                    field: None,
                    label: k.clone(),
                    dots: v.as_i64().unwrap_or(0) as i32,
                });
            }
        }
    }
    out
}

/// Resolve one trait token to `(label, dots, field_id)`, fuzzy prefix match.
fn resolve_trait(
    schema: &SheetSchema,
    doc: &SheetDoc,
    token: &str,
) -> Result<(String, i32, Option<String>), String> {
    let q = norm(token);
    if q.is_empty() {
        return Err(format!("{token:?} is not a trait"));
    }
    let cands = candidates(schema, doc);
    // Exact id/label match wins outright.
    let exact: Vec<&Candidate> = cands
        .iter()
        .filter(|c| {
            c.field.as_deref().map(norm).as_deref() == Some(q.as_str()) || norm(&c.label) == q
        })
        .collect();
    let chosen: Vec<&Candidate> = if !exact.is_empty() {
        exact
    } else {
        cands
            .iter()
            .filter(|c| {
                c.field.as_deref().is_some_and(|f| norm(f).starts_with(&q))
                    || norm(&c.label).starts_with(&q)
            })
            .collect()
    };
    match chosen.as_slice() {
        [] => Err(format!("no trait on your sheet matches {token:?}")),
        [c] => Ok((c.label.clone(), c.dots, c.field.clone())),
        many => {
            let mut names: Vec<String> = many.iter().map(|c| c.label.clone()).collect();
            names.sort();
            names.dedup();
            if names.len() == 1 {
                let c = many[0];
                Ok((c.label.clone(), c.dots, c.field.clone()))
            } else {
                Err(format!("{token:?} is ambiguous ({})", names.join(", ")))
            }
        }
    }
}

/// Split an expression into `(sign, token)` pairs on `+`/`-`.
fn split_terms(expr: &str) -> Vec<(i32, String)> {
    let mut terms = Vec::new();
    let mut sign = 1i32;
    let mut cur = String::new();
    let flush = |sign: i32, cur: &mut String, terms: &mut Vec<(i32, String)>| {
        let t = cur.trim().to_string();
        if !t.is_empty() {
            terms.push((sign, t));
        }
        cur.clear();
    };
    for ch in expr.chars() {
        match ch {
            '+' => {
                flush(sign, &mut cur, &mut terms);
                sign = 1;
            }
            '-' => {
                flush(sign, &mut cur, &mut terms);
                sign = -1;
            }
            _ => cur.push(ch),
        }
    }
    flush(sign, &mut cur, &mut terms);
    terms
}

/// Resolve a pool expression against a sheet.
///
/// # Errors
///
/// Returns a player-readable message if a trait token cannot be resolved.
pub fn resolve_pool(
    schema: &SheetSchema,
    doc: &SheetDoc,
    expr: &str,
) -> Result<ResolvedPool, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("Roll what? Give a pool like strength+brawl or a number.".to_string());
    }
    // A bare integer is a raw pool.
    if let Ok(n) = expr.parse::<i32>() {
        return Ok(ResolvedPool {
            terms: vec![PoolTerm {
                label: "pool".to_string(),
                value: n,
            }],
            total: n,
            trait_fields: Vec::new(),
        });
    }

    let mut terms = Vec::new();
    let mut trait_fields = Vec::new();
    for (sign, token) in split_terms(expr) {
        if let Ok(n) = token.parse::<i32>() {
            let value = sign * n;
            terms.push(PoolTerm {
                label: format!("{value:+}"),
                value,
            });
        } else if let Some((name, val)) = token.split_once(':') {
            let n: i32 = val
                .trim()
                .parse()
                .map_err(|_| format!("{token:?}: {val:?} is not a number"))?;
            terms.push(PoolTerm {
                label: name.trim().to_string(),
                value: sign * n,
            });
        } else {
            let (label, dots, field) = resolve_trait(schema, doc, &token)?;
            if let Some(f) = field {
                trait_fields.push(f);
            }
            terms.push(PoolTerm {
                label,
                value: sign * dots,
            });
        }
    }
    let total = terms.iter().map(|t| t.value).sum();
    Ok(ResolvedPool {
        terms,
        total,
        trait_fields,
    })
}

/// Resolve a pool expression for an **NPC ability** (`wod_roll`): like
/// [`resolve_pool`], but every term must be a real trait on the NPC's own
/// sheet — literal integers and `name:value` bonuses are rejected.
///
/// This is the over-claim fence (design 03 §7): because each term resolves to
/// the dots actually recorded on the sheet, an NPC whose sheet says `Brawl 2`
/// cannot manufacture a pool of 9 by naming `brawl:9` or `strength+brawl+7` —
/// those are refused. A flat situational bonus rides the ability's separate,
/// bounded `modifier` argument instead, never the pool expression.
///
/// # Errors
///
/// Returns a message if the expression is empty, names a term that is not a
/// sheet trait (raw dice / named bonus), names a trait the sheet does not have,
/// or names the same trait more than once (a padded pool: `arete+arete` would
/// otherwise double the dice past what the sheet holds).
pub fn resolve_sheet_pool(
    schema: &SheetSchema,
    doc: &SheetDoc,
    expr: &str,
) -> Result<ResolvedPool, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("empty pool: name traits like strength+brawl".to_string());
    }
    let mut terms = Vec::new();
    let mut trait_fields = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (sign, token) in split_terms(expr) {
        if token.parse::<i32>().is_ok() {
            return Err(format!(
                "{token:?}: an NPC roll may only name traits on its own sheet, not raw dice"
            ));
        }
        if token.contains(':') {
            return Err(format!(
                "{token:?}: named dice bonuses are not allowed in an NPC roll"
            ));
        }
        let (label, dots, field) = resolve_trait(schema, doc, &token)?;
        // A sheet pool may sum each trait at most once — the same trait named
        // twice (directly or via two tokens that resolve to it) is a padded
        // pool that would inflate the dice past the sheet. Reject it.
        let key = field.clone().unwrap_or_else(|| label.to_ascii_lowercase());
        if seen.contains(&key) {
            return Err(format!(
                "{label:?} is named more than once — a pool may sum each trait only once"
            ));
        }
        seen.push(key);
        if let Some(f) = field {
            trait_fields.push(f);
        }
        terms.push(PoolTerm {
            label,
            value: sign * dots,
        });
    }
    let total = terms.iter().map(|t| t.value).sum();
    Ok(ResolvedPool {
        terms,
        total,
        trait_fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Mortal, Template, build_schema};

    fn sheet() -> (SheetSchema, SheetDoc) {
        let schema = build_schema("wod", &[Box::new(Mortal) as Box<dyn Template>]);
        let mut doc = SheetDoc::new_default(&schema);
        doc.set_number("strength", 3);
        doc.set_number("dexterity", 2);
        doc.set_number("brawl", 2);
        doc.set_number("firearms", 4);
        doc.map_set("backgrounds", "Resources", serde_json::json!(3));
        (schema, doc)
    }

    #[test]
    fn raw_integer_pool() {
        let (schema, doc) = sheet();
        let r = resolve_pool(&schema, &doc, "7").unwrap();
        assert_eq!(r.total, 7);
        assert!(r.trait_fields.is_empty());
    }

    #[test]
    fn attribute_plus_ability_with_modifier() {
        let (schema, doc) = sheet();
        let r = resolve_pool(&schema, &doc, "stren+brawl+2").unwrap();
        assert_eq!(r.total, 3 + 2 + 2);
        assert_eq!(r.trait_fields, vec!["strength", "brawl"]);
        assert_eq!(r.terms[0].label, "Strength");
        assert_eq!(r.terms[2].label, "+2");
    }

    #[test]
    fn subtraction_and_named_bonus() {
        let (schema, doc) = sheet();
        let r = resolve_pool(&schema, &doc, "firearms-1+aim:2").unwrap();
        // 4 - 1 + 2 = 5.
        assert_eq!(r.total, 5);
        assert_eq!(r.terms[1].value, -1);
        assert_eq!(r.terms[2].label, "aim");
    }

    #[test]
    fn background_name_resolves() {
        let (schema, doc) = sheet();
        let r = resolve_pool(&schema, &doc, "resources").unwrap();
        assert_eq!(r.total, 3);
    }

    #[test]
    fn sheet_pool_rejects_a_repeated_trait() {
        let (schema, doc) = sheet();
        // A padded pool that would otherwise double the dice past the sheet.
        assert!(resolve_sheet_pool(&schema, &doc, "strength+strength").is_err());
        // Exact-id and label forms of the same trait are also refused.
        assert!(resolve_sheet_pool(&schema, &doc, "strength+Strength").is_err());
        // A legitimate attribute + ability pool still resolves.
        let ok = resolve_sheet_pool(&schema, &doc, "strength+brawl").unwrap();
        assert_eq!(ok.total, 3 + 2);
    }

    #[test]
    fn ambiguous_and_unknown() {
        let (schema, doc) = sheet();
        // "st" matches strength, stamina, streetwise, stealth...
        assert!(resolve_pool(&schema, &doc, "st").is_err());
        assert!(resolve_pool(&schema, &doc, "zzz").is_err());
        // exact id beats prefix ambiguity.
        assert!(resolve_pool(&schema, &doc, "strength").is_ok());
    }
}
