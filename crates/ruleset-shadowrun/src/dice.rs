//! SR5 pool-expression resolution: turn `body+pistols+2` (or a raw `9`) into a
//! dice pool by resolving each trait name against the runner's sheet.
//!
//! A pool expression is a `+`/`-` chain of terms:
//!
//! - an **attribute** — one of the eight core attributes or Edge/Magic/Resonance
//!   (fuzzy prefix-matched, case-insensitive), contributing its rating;
//! - a **skill** — any SR5 active skill or skill group by name. Its rating is
//!   read from the sheet's `active_skills` map, falling back to the owning
//!   **skill group** rating, and finally to **defaulting** at −1 (only for
//!   skills that may be defaulted); a knowledge/language skill on the sheet
//!   resolves too;
//! - a bare **integer** — a flat dice modifier;
//! - a **`name:value`** term — an explicitly-valued named bonus (`smartlink:2`).
//!
//! A pool expression that is a single integer (`+roll 9`) needs no sheet.
//! Resolution is pure and returns the full per-term breakdown for the roll log,
//! plus the matched skill ids (for specializations) and the **sensible Limit**
//! inferred from the traits' categories (Physical / Mental / Social).

use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{FieldType, SheetSchema};

use crate::data::{self, ActiveSkill, SkillGroup};
use crate::{derive, schema};

/// Which derived Limit a test naturally uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    /// Physical Limit (combat, physical, vehicle skills; physical attributes).
    Physical,
    /// Mental Limit (technical, magical, resonance, knowledge; mental attributes).
    Mental,
    /// Social Limit (social skills).
    Social,
}

impl LimitKind {
    /// Parse a limit keyword (`physical`/`mental`/`social`, prefix-ok).
    #[must_use]
    pub fn parse(s: &str) -> Option<LimitKind> {
        let s = s.trim().to_ascii_lowercase();
        if "physical".starts_with(&s) && !s.is_empty() {
            Some(LimitKind::Physical)
        } else if "mental".starts_with(&s) && !s.is_empty() {
            Some(LimitKind::Mental)
        } else if "social".starts_with(&s) && !s.is_empty() {
            Some(LimitKind::Social)
        } else {
            None
        }
    }

    /// The short label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            LimitKind::Physical => "Physical",
            LimitKind::Mental => "Mental",
            LimitKind::Social => "Social",
        }
    }

    /// The numeric value of this Limit on `doc`.
    #[must_use]
    pub fn value(self, doc: &SheetDoc) -> i64 {
        let l = derive::limits(doc);
        match self {
            LimitKind::Physical => l.physical,
            LimitKind::Mental => l.mental,
            LimitKind::Social => l.social,
        }
    }
}

/// The limit a skill category naturally uses.
fn limit_for_skill_category(cat: &str) -> LimitKind {
    match cat {
        "social" => LimitKind::Social,
        "technical" | "magical" | "resonance" => LimitKind::Mental,
        // combat, physical, vehicle, knowledge, language → physical/mental
        "knowledge" | "language" => LimitKind::Mental,
        _ => LimitKind::Physical,
    }
}

/// The limit an attribute category naturally uses.
fn limit_for_attr_category(cat: &str) -> Option<LimitKind> {
    match cat {
        "physical" => Some(LimitKind::Physical),
        "mental" => Some(LimitKind::Mental),
        _ => None, // special attributes have no default limit
    }
}

/// One resolved contribution to a pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolTerm {
    /// Human label ("Body", "Pistols", "+2", "defaulting").
    pub label: String,
    /// Signed dice contributed.
    pub value: i32,
}

/// A fully resolved pool: the term breakdown, the summed total (before wound
/// penalty / modifiers, which the roll layer applies), the matched skill ids
/// (for specializations), and the inferred Limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPool {
    /// Every term, in expression order.
    pub terms: Vec<PoolTerm>,
    /// The summed dice.
    pub total: i32,
    /// The active-skill ids that resolved to real skills (for specialties).
    pub skill_fields: Vec<String>,
    /// The Limit inferred from the traits' categories (`None` for a raw pool
    /// or an all-special-attribute pool).
    pub limit: Option<LimitKind>,
}

/// Normalize a name for matching: lowercase, keep only alphanumerics.
fn norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

/// A matchable trait: an attribute, an active skill, a skill group, or a
/// knowledge/language entry on the sheet.
#[derive(Clone)]
enum CandKind {
    /// A core/special attribute (`category`).
    Attr(&'static str),
    /// An active skill.
    Skill(&'static ActiveSkill),
    /// A skill group.
    Group(&'static SkillGroup),
    /// A knowledge/language skill on the sheet (map field, key).
    Known(&'static str, String, &'static str),
}

/// A resolvable candidate: its match keys and how to score it.
struct Candidate {
    /// The primary match id/name (e.g. `"body"`, `"pistols"`).
    id: String,
    /// The human label.
    label: String,
    kind: CandKind,
}

/// Read a skill-map rating (`> 0` or `None`).
fn map_rating(doc: &SheetDoc, field: &str, key: &str) -> Option<i32> {
    doc.map(field)
        .and_then(|m| m.get(key))
        .and_then(serde_json::Value::as_i64)
        .map(|n| n as i32)
        .filter(|n| *n > 0)
}

impl Candidate {
    /// Resolve this candidate against `doc`: `(label, value, skill_field,
    /// limit)`. `skill_field` is `Some(id)` for a real skill (specialties).
    fn resolve(
        &self,
        doc: &SheetDoc,
    ) -> Result<(String, i32, Option<String>, Option<LimitKind>), String> {
        match &self.kind {
            CandKind::Attr(cat) => {
                let v = doc.number(&self.id).unwrap_or(0) as i32;
                Ok((self.label.clone(), v, None, limit_for_attr_category(cat)))
            }
            CandKind::Skill(sk) => {
                let limit = Some(limit_for_skill_category(sk.category));
                // Direct rating, then skill-group fallback.
                if let Some(r) = map_rating(doc, schema::ACTIVE_SKILLS_FIELD, sk.id) {
                    return Ok((self.label.clone(), r, Some(sk.id.to_string()), limit));
                }
                if let Some(g) = data::SKILL_GROUPS
                    .iter()
                    .find(|g| g.skills.contains(&sk.id))
                    && let Some(r) = map_rating(doc, schema::SKILL_GROUPS_FIELD, g.id)
                {
                    return Ok((self.label.clone(), r, Some(sk.id.to_string()), limit));
                }
                // Untrained: default at −1, or refuse.
                if sk.defaults {
                    Ok((
                        format!("{} (defaulting)", self.label),
                        -1,
                        Some(sk.id.to_string()),
                        limit,
                    ))
                } else {
                    Err(format!(
                        "{} cannot be defaulted; you have no rating in it.",
                        self.label
                    ))
                }
            }
            CandKind::Group(g) => {
                let limit = g
                    .skills
                    .first()
                    .and_then(|s| data::active_skill(s))
                    .map(|s| limit_for_skill_category(s.category));
                let r = map_rating(doc, schema::SKILL_GROUPS_FIELD, g.id).unwrap_or(0);
                Ok((self.label.clone(), r, None, limit))
            }
            CandKind::Known(field, key, cat) => {
                let r = map_rating(doc, field, key).unwrap_or(0);
                Ok((
                    self.label.clone(),
                    r,
                    None,
                    Some(limit_for_skill_category(cat)),
                ))
            }
        }
    }
}

/// Every matchable trait: attributes (from the schema), the whole active-skill
/// and skill-group catalogue (rollable even untrained, by defaulting), plus the
/// knowledge/language entries actually on the sheet.
fn candidates(schema: &SheetSchema, doc: &SheetDoc) -> Vec<Candidate> {
    let mut out = Vec::new();
    // Attributes: every Dots/Number field that is an attribute or special.
    for (id, label, cat) in data::CORE_ATTRS {
        let _ = schema; // attributes are canonical from data
        out.push(Candidate {
            id: (*id).to_string(),
            label: (*label).to_string(),
            kind: CandKind::Attr(cat),
        });
    }
    for (id, label) in [
        (schema::EDGE, "Edge"),
        (schema::MAGIC, "Magic"),
        (schema::RESONANCE, "Resonance"),
    ] {
        out.push(Candidate {
            id: id.to_string(),
            label: label.to_string(),
            kind: CandKind::Attr("special"),
        });
    }
    // The full active-skill catalogue.
    for sk in data::ACTIVE_SKILLS {
        out.push(Candidate {
            id: sk.id.to_string(),
            label: sk.name.to_string(),
            kind: CandKind::Skill(sk),
        });
    }
    // Skill groups.
    for g in data::SKILL_GROUPS {
        out.push(Candidate {
            id: g.id.to_string(),
            label: g.name.to_string(),
            kind: CandKind::Group(g),
        });
    }
    // Knowledge / language entries on the sheet.
    for (field, cat) in [
        (schema::KNOWLEDGE_SKILLS_FIELD, "knowledge"),
        (schema::LANGUAGE_SKILLS_FIELD, "language"),
    ] {
        if let Some(m) = doc.map(field) {
            for k in m.keys() {
                out.push(Candidate {
                    id: k.clone(),
                    label: k.clone(),
                    kind: CandKind::Known(field, k.clone(), cat),
                });
            }
        }
    }
    out
}

/// Resolve one trait token to a candidate (fuzzy prefix match; exact id/label
/// wins outright).
fn match_candidate<'c>(cands: &'c [Candidate], token: &str) -> Result<&'c Candidate, String> {
    let q = norm(token);
    if q.is_empty() {
        return Err(format!("{token:?} is not a trait"));
    }
    // Exact id/label match.
    let exact: Vec<&Candidate> = cands
        .iter()
        .filter(|c| norm(&c.id) == q || norm(&c.label) == q)
        .collect();
    let chosen: Vec<&Candidate> = if !exact.is_empty() {
        exact
    } else {
        cands
            .iter()
            .filter(|c| norm(&c.id).starts_with(&q) || norm(&c.label).starts_with(&q))
            .collect()
    };
    match chosen.as_slice() {
        [] => Err(format!("no trait on your sheet matches {token:?}")),
        [c] => Ok(c),
        many => {
            let mut names: Vec<String> = many.iter().map(|c| c.label.clone()).collect();
            names.sort();
            names.dedup();
            if names.len() == 1 {
                Ok(many[0])
            } else {
                names.truncate(6);
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

/// The inferred Limit for a set of terms: the first skill's limit wins; else the
/// first attribute's; else none.
fn infer_limit(limits: &[Option<LimitKind>], skill_seen: &[bool]) -> Option<LimitKind> {
    // Prefer a skill-derived limit.
    for (i, l) in limits.iter().enumerate() {
        if skill_seen.get(i).copied().unwrap_or(false)
            && let Some(k) = l
        {
            return Some(*k);
        }
    }
    limits.iter().flatten().copied().next()
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
        return Err("Roll what? Give a pool like body+pistols or a number.".to_string());
    }
    // A bare integer is a raw pool.
    if let Ok(n) = expr.parse::<i32>() {
        return Ok(ResolvedPool {
            terms: vec![PoolTerm {
                label: "pool".to_string(),
                value: n,
            }],
            total: n,
            skill_fields: Vec::new(),
            limit: None,
        });
    }

    let cands = candidates(schema, doc);
    let mut terms = Vec::new();
    let mut skill_fields = Vec::new();
    let mut limits = Vec::new();
    let mut skill_seen = Vec::new();
    for (sign, token) in split_terms(expr) {
        if let Ok(n) = token.parse::<i32>() {
            let value = sign * n;
            terms.push(PoolTerm {
                label: format!("{value:+}"),
                value,
            });
            limits.push(None);
            skill_seen.push(false);
        } else if let Some((name, val)) = token.split_once(':') {
            let n: i32 = val
                .trim()
                .parse()
                .map_err(|_| format!("{token:?}: {val:?} is not a number"))?;
            terms.push(PoolTerm {
                label: name.trim().to_string(),
                value: sign * n,
            });
            limits.push(None);
            skill_seen.push(false);
        } else {
            let cand = match_candidate(&cands, &token)?;
            let (label, value, field, limit) = cand.resolve(doc)?;
            let is_skill = field.is_some();
            if let Some(f) = field {
                skill_fields.push(f);
            }
            terms.push(PoolTerm {
                label,
                value: sign * value,
            });
            limits.push(limit);
            skill_seen.push(is_skill);
        }
    }
    let total = terms.iter().map(|t| t.value).sum();
    let limit = infer_limit(&limits, &skill_seen);
    Ok(ResolvedPool {
        terms,
        total,
        skill_fields,
        limit,
    })
}

/// Resolve a pool for an **NPC ability** (`sr_roll`): like [`resolve_pool`], but
/// every term must be a real trait — literal dice and `name:value` bonuses are
/// rejected. This is the over-claim fence (design 03 §7): each term scores the
/// rating actually recorded on the NPC's sheet, so a model cannot manufacture a
/// pool of 12 by naming `pistols:12` or `body+pistols+8`.
///
/// # Errors
///
/// Returns a message if the expression is empty, names raw dice / a named bonus,
/// or names a trait the sheet cannot roll.
pub fn resolve_sheet_pool(
    schema: &SheetSchema,
    doc: &SheetDoc,
    expr: &str,
) -> Result<ResolvedPool, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("empty pool: name traits like body+pistols".to_string());
    }
    let cands = candidates(schema, doc);
    let mut terms = Vec::new();
    let mut skill_fields = Vec::new();
    let mut limits = Vec::new();
    let mut skill_seen = Vec::new();
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
        let cand = match_candidate(&cands, &token)?;
        let (label, value, field, limit) = cand.resolve(doc)?;
        let is_skill = field.is_some();
        if let Some(f) = field {
            skill_fields.push(f);
        }
        terms.push(PoolTerm {
            label,
            value: sign * value,
        });
        limits.push(limit);
        skill_seen.push(is_skill);
    }
    let total = terms.iter().map(|t| t.value).sum();
    let limit = infer_limit(&limits, &skill_seen);
    Ok(ResolvedPool {
        terms,
        total,
        skill_fields,
        limit,
    })
}

/// Whether any of `skill_fields` records a specialization matching `name`
/// (adds +2 dice). Used by `+roll spec <name>`.
#[must_use]
pub fn specialization_matches(doc: &SheetDoc, skill_fields: &[String], name: &str) -> bool {
    let Some(specs) = doc.map(schema::SKILL_SPECIALIZATIONS_FIELD) else {
        return false;
    };
    skill_fields.iter().any(|f| {
        specs
            .get(f)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|s| s.eq_ignore_ascii_case(name))
            })
    })
}

/// Whether any of `skill_fields` records *any* specialization (the gate for a
/// bare `/spec` request).
#[must_use]
pub fn has_any_specialization(doc: &SheetDoc, skill_fields: &[String]) -> bool {
    let Some(specs) = doc.map(schema::SKILL_SPECIALIZATIONS_FIELD) else {
        return false;
    };
    skill_fields.iter().any(|f| {
        specs
            .get(f)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|arr| arr.iter().any(serde_json::Value::is_string))
    })
}

/// The `FieldType` of a schema field id (used by staff edits; kept crate-local).
#[allow(dead_code)]
pub(crate) fn field_type<'s>(schema: &'s SheetSchema, id: &str) -> Option<&'s FieldType> {
    schema.field(id).map(|f| &f.ty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::build_schema;
    use mush_ruleset::SheetDoc;

    fn sheet() -> (SheetSchema, SheetDoc) {
        let schema = build_schema("shadowrun");
        let mut doc = SheetDoc::new_default(&schema);
        doc.set_number("body", 5);
        doc.set_number("agility", 6);
        doc.set_number("charisma", 3);
        doc.map_set(schema::ACTIVE_SKILLS_FIELD, "pistols", serde_json::json!(4));
        doc.map_set(schema::SKILL_GROUPS_FIELD, "firearms", serde_json::json!(3));
        doc.map_set(schema::ACTIVE_SKILLS_FIELD, "con", serde_json::json!(2));
        (schema, doc)
    }

    #[test]
    fn raw_integer_pool() {
        let (s, d) = sheet();
        let r = resolve_pool(&s, &d, "9").unwrap();
        assert_eq!(r.total, 9);
        assert!(r.limit.is_none());
    }

    #[test]
    fn attribute_plus_skill_with_modifier() {
        let (s, d) = sheet();
        let r = resolve_pool(&s, &d, "agility+pistols+2").unwrap();
        assert_eq!(r.total, 6 + 4 + 2);
        assert_eq!(r.skill_fields, vec!["pistols"]);
        assert_eq!(r.limit, Some(LimitKind::Physical));
    }

    #[test]
    fn skill_group_fallback() {
        let (s, d) = sheet();
        // automatics not bought directly, but firearms group is 3.
        let r = resolve_pool(&s, &d, "agility+automatics").unwrap();
        assert_eq!(r.total, 6 + 3);
    }

    #[test]
    fn defaulting_applies_minus_one() {
        let (s, d) = sheet();
        // gymnastics: defaultable, no rating, no group → −1.
        let r = resolve_pool(&s, &d, "agility+gymnastics").unwrap();
        assert_eq!(r.total, 6 - 1);
        assert!(r.terms.iter().any(|t| t.label.contains("defaulting")));
    }

    #[test]
    fn non_defaultable_skill_errors() {
        let (s, d) = sheet();
        // alchemy: defaults = false, no rating.
        assert!(resolve_pool(&s, &d, "magic+alchemy").is_err());
    }

    #[test]
    fn social_skill_infers_social_limit() {
        let (s, d) = sheet();
        let r = resolve_pool(&s, &d, "charisma+con").unwrap();
        assert_eq!(r.limit, Some(LimitKind::Social));
    }

    #[test]
    fn sheet_pool_rejects_raw_dice() {
        let (s, d) = sheet();
        assert!(resolve_sheet_pool(&s, &d, "agility+5").is_err());
        assert!(resolve_sheet_pool(&s, &d, "pistols:9").is_err());
        assert!(resolve_sheet_pool(&s, &d, "agility+pistols").is_ok());
    }
}
