//! SR5 **priority** chargen accounting and the [`ChargenFlow`] implementation.
//!
//! The classic SR5 priority system: assign A–E (each once) across five
//! categories — Metatype / Attributes / Magic-Resonance / Skills / Resources —
//! each letter yielding a different allotment ([`crate::data`]). Then spend the
//! allotments plus a 25-Karma finishing pool.
//!
//! The budget ledgers enforced (against the priority assignment persisted on the
//! sheet, reboot-safe):
//!
//! | pool | allotment | spent |
//! |---|---|---|
//! | Attributes | 24/20/16/14/12 | Σ(core attr − metatype min) |
//! | Special | metatype special points | Σ Edge/Magic/Resonance above base |
//! | Active Skills | 46/36/28/22/18 | Σ active-skill ratings − free magical cover |
//! | Magical Skills | free count × rating | magical-skill spend the free budget covers |
//! | Skill Groups | 10/5/2/0/0 | Σ skill-group ratings |
//! | Knowledge | (Logic+Intuition)×2 | Σ knowledge + language ratings |
//! | Spells / Complex Forms | Magic-priority free count | learned count (extras → Karma) |
//! | Contacts | Charisma × 3 | Σ(connection + loyalty) (extras → Karma) |
//! | Resources | 450k…6k ¥ | current `nuyen` |
//! | Karma | 25 (+ negative qualities, ≤25) | Σ positive (≤25) + extra spells/forms/contacts |
//!
//! Caps: each core attribute sits within its metatype `[min, max]` (a below-min
//! floor is seeded when the metatype is chosen and re-checked here), only **one**
//! attribute may sit at its natural max at chargen, active skills / groups ≤ 6,
//! Edge ≤ the metatype Edge max, and Magic / Resonance ≤ 6 at creation.
//! Magic/Resonance must be consistent with the Magic priority + awakened type.
//! (Starting nuyen is **not** capped at the RAW 5,000¥ carry-over: gear is not
//! priced here, so `nuyen` is the resource allotment, not post-shopping cash.)

use mush_ruleset::SheetDoc;
use mush_ruleset::chargen::{ChargenFlow, ChargenIssue, ChargenStage};
use smol_str::SmolStr;

use crate::data::{self, Priority, Sr5Config};
use crate::schema::{
    ACTIVE_SKILLS_FIELD, CHARGEN_PRIORITIES_FIELD, EDGE, EDGE_CURRENT, ESSENCE,
    KNOWLEDGE_SKILLS_FIELD, LANGUAGE_SKILLS_FIELD, MAGIC, RESONANCE, SKILL_GROUPS_FIELD,
};

/// The five priority categories, in canonical order.
pub const PRIORITY_CATEGORIES: [&str; 5] =
    ["metatype", "attributes", "magic", "skills", "resources"];

/// The chargen-time rating cap for a core attribute above the metatype minimum
/// is the metatype natural max; active skills / groups cap here.
pub const SKILL_CHARGEN_CAP: i32 = 6;

/// The character-creation ceiling for Magic / Resonance (initiation and
/// submersion raise them past this in play, never at chargen).
pub const SPECIAL_CHARGEN_MAX: i32 = 6;

/// The independent character-creation cap on Karma spent on positive qualities
/// (SR5 core p.72; the same cap bounds negative-quality Karma returned).
pub const POSITIVE_QUALITY_KARMA_CAP: i32 = 25;

/// A resolved priority assignment (`category -> letter`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Priorities {
    /// `[(category, letter), …]`.
    pub map: Vec<(String, Priority)>,
}

impl Priorities {
    /// The letter for `category`.
    #[must_use]
    pub fn get(&self, category: &str) -> Option<Priority> {
        self.map
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(category))
            .map(|(_, p)| *p)
    }

    /// Whether all five categories are assigned.
    #[must_use]
    pub fn complete(&self) -> bool {
        PRIORITY_CATEGORIES.iter().all(|c| self.get(c).is_some())
    }

    /// Whether the five letters form a valid A–E permutation.
    #[must_use]
    pub fn is_permutation(&self) -> bool {
        if !self.complete() {
            return false;
        }
        let mut seen = [false; 5];
        for (_, p) in &self.map {
            let i = p.index();
            if seen[i] {
                return false;
            }
            seen[i] = true;
        }
        seen.iter().all(|&b| b)
    }
}

/// Read the priority assignment persisted on the sheet's staff-only
/// `chargen_priorities` map (empty if none recorded).
#[must_use]
pub fn persisted_priorities(doc: &SheetDoc) -> Priorities {
    let mut map = Vec::new();
    if let Some(m) = doc.map(CHARGEN_PRIORITIES_FIELD) {
        for cat in PRIORITY_CATEGORIES {
            if let Some(letter) = m.get(cat).and_then(serde_json::Value::as_str)
                && let Some(p) = letter.chars().next().and_then(Priority::from_char)
            {
                map.push((cat.to_string(), p));
            }
        }
    }
    Priorities { map }
}

/// The chosen metatype (default human).
fn metatype(doc: &SheetDoc) -> &'static data::Metatype {
    data::metatype(doc.text("metatype").unwrap_or("human")).unwrap_or(&data::METATYPES[0])
}

/// Raise every core attribute (and Edge) that sits below the sheet's metatype
/// minimum up to that minimum, never lowering an attribute already at or above
/// it. Called when the metatype is chosen and again at finalize so a runner
/// always starts at its racial floor (a Troll's Body/Strength 5, an Elf's
/// Agility 2, …) — the schema default of `1` would otherwise leave a Troll at
/// Body 1. Returns whether anything changed.
pub fn seed_metatype_minimums(doc: &mut SheetDoc) -> bool {
    let mt = metatype(doc);
    let mut changed = false;
    for (id, _, _) in data::CORE_ATTRS {
        if let Some((min, _)) = mt.attr_range(id)
            && doc.number(id).unwrap_or(0) < i64::from(min)
        {
            doc.set_number(id, i64::from(min));
            changed = true;
        }
    }
    if doc.number(EDGE).unwrap_or(0) < i64::from(mt.edge_range.0) {
        doc.set_number(EDGE, i64::from(mt.edge_range.0));
        changed = true;
    }
    changed
}

/// The awakened type id (default mundane).
fn awakened(doc: &SheetDoc) -> String {
    doc.text("awakened_type").unwrap_or("mundane").to_string()
}

/// The Magic rating the Magic priority grants for the sheet's awakened type.
#[must_use]
pub fn magic_base(doc: &SheetDoc, p_magic: Priority) -> i64 {
    let row = data::magic_priority(p_magic);
    match awakened(doc).as_str() {
        "magician" | "mystic_adept" => i64::from(row.magician_magic),
        "adept" => i64::from(row.adept_magic),
        "aspected_magician" => i64::from(row.aspected_magic),
        _ => 0,
    }
}

/// The Resonance rating the Magic priority grants for a technomancer.
#[must_use]
pub fn resonance_base(doc: &SheetDoc, p_magic: Priority) -> i64 {
    if awakened(doc) == "technomancer" {
        i64::from(data::magic_priority(p_magic).technomancer_resonance)
    } else {
        0
    }
}

/// Sum of a numeric map field's values (`> 0`).
fn map_sum(doc: &SheetDoc, field: &str) -> i64 {
    doc.map(field)
        .map(|m| m.values().filter_map(serde_json::Value::as_i64).sum())
        .unwrap_or(0)
}

/// A budget ledger for one chargen pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ledger {
    /// Pool id.
    pub name: SmolStr,
    /// Human label.
    pub label: String,
    /// Points available.
    pub total: i64,
    /// Points spent.
    pub spent: i64,
    /// Whether leftover points are allowed (only overspend is an issue).
    pub underspend_ok: bool,
    /// Whether overspend is allowed (an *informational* ledger whose overspend
    /// is paid for elsewhere — e.g. extra spells/contacts draw from Karma, so
    /// the count is shown but never itself flagged).
    pub overspend_ok: bool,
}

impl Ledger {
    fn new(name: &str, label: &str, total: i64, spent: i64, underspend_ok: bool) -> Ledger {
        Ledger {
            name: SmolStr::new(name),
            label: label.to_string(),
            total,
            spent,
            underspend_ok,
            overspend_ok: false,
        }
    }

    /// Mark this ledger informational: its overspend is paid for elsewhere
    /// (Karma) and must not itself raise an [`Overspent`](ChargenIssue::Overspent)
    /// issue.
    fn informational(mut self) -> Ledger {
        self.overspend_ok = true;
        self.underspend_ok = true;
        self
    }
}

/// Compute all budget ledgers for a chargen sheet against `priorities`.
#[must_use]
pub fn ledgers(config: &Sr5Config, doc: &SheetDoc, priorities: &Priorities) -> Vec<Ledger> {
    let mt = metatype(doc);
    let mut out = Vec::new();

    // Attributes.
    let p_attr = priorities.get("attributes").unwrap_or(Priority::E);
    let attr_total = i64::from(config.attribute_points(p_attr));
    let attr_spent: i64 = data::CORE_ATTRS
        .iter()
        .map(|(id, _, _)| {
            let min = mt.attr_range(id).map(|(lo, _)| i64::from(lo)).unwrap_or(1);
            (doc.number(id).unwrap_or(min) - min).max(0)
        })
        .sum();
    out.push(Ledger::new(
        "attributes",
        "Attributes",
        attr_total,
        attr_spent,
        false,
    ));

    // Special attributes (metatype priority points).
    let p_mt = priorities.get("metatype").unwrap_or(Priority::E);
    let special_total = i64::from(data::metatype_special_points(p_mt, mt.id).unwrap_or(0));
    let edge_min = i64::from(mt.edge_range.0);
    let p_magic = priorities.get("magic").unwrap_or(Priority::E);
    let mbase = magic_base(doc, p_magic);
    let rbase = resonance_base(doc, p_magic);
    let special_spent = (doc.number(EDGE).unwrap_or(edge_min) - edge_min).max(0)
        + (doc.number(MAGIC).unwrap_or(0) - mbase).max(0)
        + (doc.number(RESONANCE).unwrap_or(0) - rbase).max(0);
    out.push(Ledger::new(
        "special",
        "Special (Edge/Magic/Res)",
        special_total,
        special_spent,
        true,
    ));

    // Magic-priority free allotments: the Magician path's free spells + free
    // Magical skills, and the technomancer's free complex forms. These are
    // "already paid for" by the Magic priority; anything beyond them draws Karma.
    let magic = MagicAllotment::compute(doc, p_magic);

    // Skills + skill groups. The free Magical-skill points (Magician/Mystic
    // Adept) pay for the first `magic.magical_bonus` points of magical-category
    // active skills, so they do not consume the Skills priority pool.
    let p_skills = priorities.get("skills").unwrap_or(Priority::E);
    let (sk_total, grp_total) = config.skill_points(p_skills);
    let active_spent = (map_sum(doc, ACTIVE_SKILLS_FIELD) - magic.magical_covered).max(0);
    out.push(Ledger::new(
        "skills",
        "Active Skills",
        i64::from(sk_total),
        active_spent,
        false,
    ));
    if magic.magical_bonus > 0 {
        out.push(
            Ledger::new(
                "magical_skills",
                "Magical Skills (free)",
                magic.magical_bonus,
                magic.magical_covered,
                true,
            )
            .informational(),
        );
    }
    out.push(Ledger::new(
        "skill_groups",
        "Skill Groups",
        i64::from(grp_total),
        map_sum(doc, SKILL_GROUPS_FIELD),
        false,
    ));

    // Knowledge / language (Logic + Intuition) × 2.
    let know_total = (doc.number("logic").unwrap_or(1) + doc.number("intuition").unwrap_or(1)) * 2;
    let know_spent = map_sum(doc, KNOWLEDGE_SKILLS_FIELD) + map_sum(doc, LANGUAGE_SKILLS_FIELD);
    out.push(Ledger::new(
        "knowledge",
        "Knowledge/Language",
        know_total,
        know_spent,
        true,
    ));

    // Spells / complex forms: bounded by the Magic-priority free count; the
    // display ledger never itself errors (extra spells/forms cost Karma below).
    if magic.free_spells > 0 || magic.spell_count > 0 {
        out.push(
            Ledger::new(
                "spells",
                "Spells (free)",
                magic.free_spells,
                magic.spell_count,
                true,
            )
            .informational(),
        );
    }
    if magic.free_forms > 0 || magic.form_count > 0 {
        out.push(
            Ledger::new(
                "complex_forms",
                "Complex Forms (free)",
                magic.free_forms,
                magic.form_count,
                true,
            )
            .informational(),
        );
    }

    // Contacts: a free pool of Charisma × 3 Karma; the rest is bought with Karma.
    let contact_points = map_sum(doc, "contacts_connection") + map_sum(doc, "contacts_loyalty");
    let contact_free = doc.number("charisma").unwrap_or(1).max(0) * 3;
    out.push(
        Ledger::new("contacts", "Contacts", contact_free, contact_points, true).informational(),
    );

    // Resources (nuyen). Underspend allowed (leftover cash). Gear is not priced
    // in this milestone (a Map of owned items, not a costed catalogue), so the
    // SR5 5,000¥ carry-over cap is deliberately not enforced — `nuyen` here is
    // the starting-resource allotment itself, not post-shopping liquid cash.
    let p_res = priorities.get("resources").unwrap_or(Priority::E);
    let res_total = i64::from(config.resource_nuyen(p_res));
    let nuyen = doc.number("nuyen").unwrap_or(0);
    out.push(Ledger::new(
        "resources",
        "Resources (¥)",
        res_total,
        nuyen,
        true,
    ));

    // Karma: 25 + negative-quality Karma (capped at 25 bonus), spent on positive
    // qualities plus anything drawn past a free allotment — extra spells
    // (×spell), extra complex forms (×complex_form), and contacts past the
    // Charisma × 3 free pool (1 Karma per point).
    let neg = map_sum(doc, "negative_qualities").min(25);
    let karma_total = i64::from(config.chargen_karma) + neg;
    let extra_spell_karma =
        (magic.spell_count - magic.free_spells).max(0) * i64::from(config.karma.spell);
    let extra_form_karma =
        (magic.form_count - magic.free_forms).max(0) * i64::from(config.karma.complex_form);
    let extra_contact_karma = (contact_points - contact_free).max(0);
    let karma_spent = map_sum(doc, "positive_qualities")
        + extra_spell_karma
        + extra_form_karma
        + extra_contact_karma;
    out.push(Ledger::new(
        "karma",
        "Karma",
        karma_total,
        karma_spent,
        true,
    ));

    out
}

/// The Magic-priority free allotments realised against a sheet: free spells /
/// complex forms and the free Magical-skill budget (Magician / Mystic Adept).
struct MagicAllotment {
    /// Free spells the Magic priority grants (Magician / Mystic Adept only).
    free_spells: i64,
    /// Spells currently on the sheet.
    spell_count: i64,
    /// Free complex forms the Magic priority grants (Technomancer only).
    free_forms: i64,
    /// Complex forms currently on the sheet.
    form_count: i64,
    /// Free Magical-skill points (count × rating; Magician / Mystic Adept only).
    magical_bonus: i64,
    /// How much of the magical-category active-skill spend the free budget
    /// covers (`min(magical_spent, magical_bonus)`).
    magical_covered: i64,
}

impl MagicAllotment {
    fn compute(doc: &SheetDoc, p_magic: Priority) -> MagicAllotment {
        let aw = awakened(doc);
        let row = data::magic_priority(p_magic);
        // Free spells + free Magical skills are the Magician-path grant.
        let is_full_mage = matches!(aw.as_str(), "magician" | "mystic_adept");
        let free_spells = if is_full_mage {
            i64::from(row.free_spells)
        } else {
            0
        };
        let magical_bonus = if is_full_mage {
            i64::from(row.free_magic_skill_count) * i64::from(row.free_magic_skill_rating)
        } else {
            0
        };
        let free_forms = if aw == "technomancer" {
            i64::from(row.complex_forms)
        } else {
            0
        };
        let magical_spent = magical_skill_spent(doc);
        MagicAllotment {
            free_spells,
            spell_count: list_len(doc, "spells"),
            free_forms,
            form_count: list_len(doc, "complex_forms"),
            magical_bonus,
            magical_covered: magical_spent.min(magical_bonus),
        }
    }
}

/// The number of entries in a list field (spells / complex forms).
fn list_len(doc: &SheetDoc, field: &str) -> i64 {
    doc.list(field).map(|l| l.len() as i64).unwrap_or(0)
}

/// Total rating spent on magical-category active skills (the skills the free
/// Magical-skill budget can pay for).
fn magical_skill_spent(doc: &SheetDoc) -> i64 {
    doc.map(ACTIVE_SKILLS_FIELD)
        .map(|m| {
            m.iter()
                .filter(|(k, _)| data::active_skill(k).is_some_and(|s| s.category == "magical"))
                .filter_map(|(_, v)| v.as_i64())
                .sum()
        })
        .unwrap_or(0)
}

/// The outstanding chargen issues: over/under-spend, attribute/skill caps, and
/// Magic-priority / awakened consistency.
#[must_use]
pub fn issues(config: &Sr5Config, doc: &SheetDoc, priorities: &Priorities) -> Vec<ChargenIssue> {
    let mut issues = Vec::new();

    if !priorities.complete() {
        issues.push(ChargenIssue::msg(
            "Assign all five priorities first (+chargen/priority).",
        ));
        return issues;
    }
    if !priorities.is_permutation() {
        issues.push(ChargenIssue::msg(
            "Priorities must use each of A, B, C, D, E exactly once.",
        ));
    }

    for l in ledgers(config, doc, priorities) {
        if l.spent > l.total && !l.overspend_ok {
            issues.push(ChargenIssue::Overspent {
                pool: l.name.clone(),
                label: l.label.clone(),
                by: (l.spent - l.total) as i32,
            });
        } else if l.spent < l.total && !l.underspend_ok {
            issues.push(ChargenIssue::Underspent {
                pool: l.name.clone(),
                label: l.label.clone(),
                by: (l.total - l.spent) as i32,
            });
        }
    }

    // Attribute natural-min floors + natural-max caps + the "one attribute at
    // max" rule. The floor stops a sub-metatype-minimum runner (a Troll with
    // Body 1) passing approval "for free" (the attribute ledger only counts
    // spend above the minimum, so a below-min value would otherwise be unpriced
    // AND unflagged).
    let mt = metatype(doc);
    let mut at_max = 0;
    for (id, label, _) in data::CORE_ATTRS {
        let (min, max) = mt.attr_range(id).unwrap_or((1, 6));
        let v = doc.number(id).unwrap_or(i64::from(min));
        if v < i64::from(min) {
            issues.push(ChargenIssue::msg(format!(
                "{label} {v} is below the {} minimum of {min}.",
                mt.name
            )));
        }
        if v > i64::from(max) {
            issues.push(ChargenIssue::AboveCap {
                field: SmolStr::new(*id),
                cap: i32::from(max),
                got: v as i32,
            });
        } else if v == i64::from(max) {
            at_max += 1;
            let _ = label;
        }
    }
    if at_max > 1 {
        issues.push(ChargenIssue::msg(
            "Only one attribute may be at its metatype maximum at character creation.",
        ));
    }

    // Special-attribute caps: Edge ≤ the metatype Edge maximum, and Magic /
    // Resonance ≤ 6 at character creation (initiation/submersion raise them
    // later, not at chargen). The special-points ledger only bounds *spend*,
    // never the ceiling, so without this a runner could set Magic 12 at chargen.
    let edge = doc.number(EDGE).unwrap_or(i64::from(mt.edge_range.0));
    if edge < i64::from(mt.edge_range.0) {
        issues.push(ChargenIssue::msg(format!(
            "Edge {edge} is below the {} minimum of {}.",
            mt.name, mt.edge_range.0
        )));
    }
    if edge > i64::from(mt.edge_range.1) {
        issues.push(ChargenIssue::AboveCap {
            field: SmolStr::new(EDGE),
            cap: i32::from(mt.edge_range.1),
            got: edge as i32,
        });
    }
    for (field, label) in [(MAGIC, "Magic"), (RESONANCE, "Resonance")] {
        let v = doc.number(field).unwrap_or(0);
        if v > i64::from(SPECIAL_CHARGEN_MAX) {
            issues.push(ChargenIssue::msg(format!(
                "{label} {v} exceeds the character-creation maximum of {SPECIAL_CHARGEN_MAX}."
            )));
        }
    }

    // Positive qualities are independently capped at 25 Karma at chargen (SR5
    // core p.72), regardless of how much Karma negative qualities returned.
    let positive = map_sum(doc, "positive_qualities");
    if positive > i64::from(POSITIVE_QUALITY_KARMA_CAP) {
        issues.push(ChargenIssue::msg(format!(
            "Positive qualities total {positive} Karma, over the character-creation \
             cap of {POSITIVE_QUALITY_KARMA_CAP}."
        )));
    }

    // Active-skill / group chargen caps (≤ 6).
    for (field, what) in [
        (ACTIVE_SKILLS_FIELD, "skill"),
        (SKILL_GROUPS_FIELD, "skill group"),
    ] {
        if let Some(m) = doc.map(field) {
            for (k, v) in m {
                if v.as_i64().unwrap_or(0) > i64::from(SKILL_CHARGEN_CAP) {
                    issues.push(ChargenIssue::msg(format!(
                        "{what} {k:?} exceeds the chargen cap of {SKILL_CHARGEN_CAP}."
                    )));
                }
            }
        }
    }

    // Magic / awakened consistency.
    let p_magic = priorities.get("magic").unwrap_or(Priority::E);
    let aw = awakened(doc);
    let mbase = magic_base(doc, p_magic);
    let rbase = resonance_base(doc, p_magic);
    match aw.as_str() {
        "mundane" => {}
        "technomancer" => {
            if rbase < 1 {
                issues.push(ChargenIssue::msg(format!(
                    "A technomancer needs a Magic priority that grants Resonance (A–C); \
                     priority {} does not.",
                    p_magic.as_char()
                )));
            }
        }
        _ => {
            if mbase < 1 {
                issues.push(ChargenIssue::msg(format!(
                    "A {aw} needs a Magic priority that grants Magic; priority {} does not.",
                    p_magic.as_char()
                )));
            }
        }
    }

    issues
}

/// Finalize: verify the budget, seed Magic/Resonance to their priority base,
/// recompute Essence from augmentations, run [`crate::derive::apply_derived`]
/// (Limits / Condition Monitors / Initiative), fill Edge to full, and set adept
/// Power Points.
///
/// # Errors
///
/// Returns the first blocking [`ChargenIssue`] if the budget does not balance.
pub fn finalize_sheet(
    config: &Sr5Config,
    doc: &mut SheetDoc,
    priorities: &Priorities,
) -> Result<(), ChargenIssue> {
    if let Some(first) = issues(config, doc, priorities).into_iter().next() {
        return Err(first);
    }
    // Guarantee the racial attribute floor is seeded (belt-and-suspenders: the
    // metatype may have been chosen through a path that did not seed it).
    seed_metatype_minimums(doc);
    let p_magic = priorities.get("magic").unwrap_or(Priority::E);
    let aw = awakened(doc);

    // Seed Magic / Resonance from the priority base (never lower them).
    let mbase = magic_base(doc, p_magic);
    if mbase > 0 {
        let m = doc.number(MAGIC).unwrap_or(0).max(mbase);
        doc.set_number(MAGIC, m);
    }
    let rbase = resonance_base(doc, p_magic);
    if rbase > 0 {
        let r = doc.number(RESONANCE).unwrap_or(0).max(rbase);
        doc.set_number(RESONANCE, r);
    }
    // Adept Power Points = Magic rating. Only a *pure* Adept gets Power Points
    // for free; a Mystic Adept buys Power Points with Karma (5 each), so it is
    // not seeded here (SR5 core p.69).
    if aw == "adept" {
        doc.set_number("power_points", doc.number(MAGIC).unwrap_or(0));
    }

    // Essence from augmentations, then all attribute-derived stats.
    let (_, essence) = crate::derive::essence_from_augmentations(doc);
    doc.set_number(ESSENCE, essence);
    crate::derive::apply_derived(doc);

    // Edge refreshes to full for the session.
    let edge = doc.number(EDGE).unwrap_or(1);
    doc.set_number(EDGE_CURRENT, edge);
    Ok(())
}

/// The SR5 priority chargen flow (framework-facing). Assesses against the
/// priority assignment persisted on the sheet, matching the live `+chargen`
/// command surface.
pub struct Sr5Chargen {
    config: Sr5Config,
}

impl Sr5Chargen {
    /// A flow over `config`.
    #[must_use]
    pub fn new(config: Sr5Config) -> Sr5Chargen {
        Sr5Chargen { config }
    }

    /// The config this flow costs against.
    #[must_use]
    pub fn config(&self) -> &Sr5Config {
        &self.config
    }
}

impl ChargenFlow for Sr5Chargen {
    fn stages(&self) -> Vec<ChargenStage> {
        vec![
            ChargenStage::new(
                "priorities",
                "Priority Assignment",
                "Assign A–E (each once) across Metatype, Attributes, Magic, Skills, \
                 Resources with +chargen/priority.",
            ),
            ChargenStage::new(
                "metatype",
                "Metatype",
                "Choose your metatype with +stat metatype=<human|elf|dwarf|ork|troll>.",
            ),
            ChargenStage::new(
                "attributes",
                "Attributes",
                "Spend your Attribute points (+stat body=5). One attribute may reach \
                 its metatype maximum.",
            ),
            ChargenStage::new(
                "magic",
                "Magic / Resonance",
                "Set awakened_type + tradition and buy Magic/Resonance, spells, or \
                 complex forms (Awakened only).",
            ),
            ChargenStage::new(
                "skills",
                "Skills",
                "Buy active skills (+stat active_skills:pistols=4), skill groups \
                 (+stat skill_groups:firearms=3), and specializations (+spec).",
            ),
            ChargenStage::new(
                "resources",
                "Resources",
                "Set starting nuyen + augmentations/gear within your Resource priority.",
            ),
            ChargenStage::new(
                "finishing",
                "Finishing Touches",
                "Spend Karma on qualities/contacts, then +chargen/submit.",
            ),
        ]
    }

    fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue> {
        let priorities = persisted_priorities(sheet);
        let all = issues(&self.config, sheet, &priorities);
        all.into_iter()
            .filter(|i| stage_owns(stage, issue_pool(i)))
            .collect()
    }

    fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue> {
        let priorities = persisted_priorities(sheet);
        finalize_sheet(&self.config, sheet, &priorities)
    }
}

/// The pool id an issue is about.
fn issue_pool(i: &ChargenIssue) -> &str {
    match i {
        ChargenIssue::Overspent { pool, .. } | ChargenIssue::Underspent { pool, .. } => {
            pool.as_str()
        }
        ChargenIssue::AboveCap { field, .. } => field.as_str(),
        _ => "",
    }
}

/// Whether `stage` owns `pool` (for per-stage validation filtering).
fn stage_owns(stage: &str, pool: &str) -> bool {
    match stage {
        "attributes" => {
            matches!(pool, "attributes" | "special")
                || data::CORE_ATTRS.iter().any(|(id, _, _)| *id == pool)
        }
        "skills" => matches!(pool, "skills" | "skill_groups" | "knowledge"),
        "resources" => pool == "resources",
        "finishing" => pool == "karma",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::build_schema;
    use mush_ruleset::SheetDoc;

    fn priorities(m: char, a: char, mg: char, s: char, r: char) -> Priorities {
        Priorities {
            map: vec![
                ("metatype".into(), Priority::from_char(m).unwrap()),
                ("attributes".into(), Priority::from_char(a).unwrap()),
                ("magic".into(), Priority::from_char(mg).unwrap()),
                ("skills".into(), Priority::from_char(s).unwrap()),
                ("resources".into(), Priority::from_char(r).unwrap()),
            ],
        }
    }

    fn doc() -> SheetDoc {
        SheetDoc::new_default(&build_schema("shadowrun"))
    }

    #[test]
    fn permutation_check() {
        assert!(priorities('A', 'B', 'C', 'D', 'E').is_permutation());
        assert!(!priorities('A', 'A', 'C', 'D', 'E').is_permutation());
    }

    #[test]
    fn attribute_ledger_counts_above_metatype_min() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human");
        // Attributes priority B = 20 points. Spend body 1→5 (4), agility 1→5 (4), rest 0.
        d.set_number("body", 5);
        d.set_number("agility", 5);
        let ls = ledgers(&cfg, &d, &priorities('C', 'B', 'D', 'A', 'E'));
        let attr = ls.iter().find(|l| l.name == "attributes").unwrap();
        assert_eq!(attr.total, 20);
        assert_eq!(attr.spent, 8);
    }

    #[test]
    fn troll_minimums_do_not_count_against_the_pool() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "troll");
        // Troll Body min 5; leaving it at 5 spends 0.
        d.set_number("body", 5);
        d.set_number("strength", 5);
        let ls = ledgers(&cfg, &d, &priorities('A', 'C', 'E', 'B', 'D'));
        let attr = ls.iter().find(|l| l.name == "attributes").unwrap();
        assert_eq!(attr.spent, 0);
    }

    #[test]
    fn skill_and_group_ledgers() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.map_set(ACTIVE_SKILLS_FIELD, "pistols", serde_json::json!(4));
        d.map_set(ACTIVE_SKILLS_FIELD, "sneaking", serde_json::json!(2));
        d.map_set(SKILL_GROUPS_FIELD, "firearms", serde_json::json!(3));
        // Skills priority A = (46, 10).
        let ls = ledgers(&cfg, &d, &priorities('C', 'B', 'E', 'A', 'D'));
        assert_eq!(ls.iter().find(|l| l.name == "skills").unwrap().spent, 6);
        assert_eq!(
            ls.iter().find(|l| l.name == "skill_groups").unwrap().spent,
            3
        );
    }

    #[test]
    fn magician_needs_a_magic_priority() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("awakened_type", "magician");
        // Magic priority E grants no Magic → an issue.
        let iss = issues(&cfg, &d, &priorities('A', 'B', 'E', 'C', 'D'));
        assert!(
            iss.iter()
                .any(|i| matches!(i, ChargenIssue::Message(m) if m.contains("Magic")))
        );
    }

    #[test]
    fn one_attribute_at_max_rule() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human"); // all maxes 6
        d.set_number("body", 6);
        d.set_number("agility", 6);
        let iss = issues(&cfg, &d, &priorities('C', 'A', 'E', 'B', 'D'));
        assert!(
            iss.iter()
                .any(|i| matches!(i, ChargenIssue::Message(m) if m.contains("maximum")))
        );
    }

    #[test]
    fn finalize_seeds_magic_and_derives() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human");
        d.set_text("awakened_type", "magician");
        // Balance every pool by giving exactly the right allotments.
        // Priorities: metatype=C(human=5 special), attributes=E(12), magic=A(magic6),
        // skills=E(18), resources=D. We just spend each pool to its total.
        let pr = priorities('C', 'E', 'A', 'D', 'B');
        // attributes E = 12: raise 3 mental attrs by 4 each = 12.
        d.set_number("logic", 5);
        d.set_number("intuition", 5);
        d.set_number("willpower", 5);
        // special C human = 5: edge 2→7 costs 5.
        d.set_number(EDGE, 7);
        // skills D = (22, 0): 22 skill points from the pool. A Magic-A Magician
        // also gets 2 free Magical skills at Rating 5 = 10 free magical points,
        // which pay for the first 10 points of magical-category skills. So spend
        // 32 magical points total: 22 from the pool + 10 covered by the free
        // Magical-skill budget.
        d.map_set(ACTIVE_SKILLS_FIELD, "spellcasting", serde_json::json!(6));
        d.map_set(ACTIVE_SKILLS_FIELD, "counterspelling", serde_json::json!(6));
        d.map_set(ACTIVE_SKILLS_FIELD, "summoning", serde_json::json!(6));
        d.map_set(ACTIVE_SKILLS_FIELD, "assensing", serde_json::json!(4));
        d.map_set(ACTIVE_SKILLS_FIELD, "banishing", serde_json::json!(6));
        d.map_set(ACTIVE_SKILLS_FIELD, "binding", serde_json::json!(4));
        // groups D = 0.
        // knowledge (Logic 5 + Intuition 5) * 2 = 20.
        d.map_set(
            KNOWLEDGE_SKILLS_FIELD,
            "Magical Theory",
            serde_json::json!(6),
        );
        d.map_set(KNOWLEDGE_SKILLS_FIELD, "Sprawl Life", serde_json::json!(6));
        d.map_set(LANGUAGE_SKILLS_FIELD, "English", serde_json::json!(4));
        d.map_set(LANGUAGE_SKILLS_FIELD, "Sperethiel", serde_json::json!(4));
        // resources: leftover cash ok; karma: leftover ok.
        let iss = issues(&cfg, &d, &pr);
        assert!(iss.is_empty(), "unexpected issues: {iss:?}");
        finalize_sheet(&cfg, &mut d, &pr).unwrap();
        // Magic seeded to 6, power points not adept, essence 6, derived present.
        assert_eq!(d.number(MAGIC), Some(6));
        assert_eq!(d.number(ESSENCE), Some(600));
        assert_eq!(d.number(EDGE_CURRENT), Some(7));
        assert!(d.number(crate::schema::PHYS_LIMIT).is_some());
    }

    /// A balanced full-mage sheet (metatype=C, attributes=E, magic=A, skills=D,
    /// resources=B) with the given awakened type — the shared fixture for the
    /// magic-path finalize tests. Priorities to pass: `('C','E','A','D','B')`.
    fn balanced_full_mage(awakened: &str) -> SheetDoc {
        let mut d = doc();
        d.set_text("metatype", "human");
        d.set_text("awakened_type", awakened);
        d.set_number("logic", 5);
        d.set_number("intuition", 5);
        d.set_number("willpower", 5);
        d.set_number(EDGE, 7);
        for (s, r) in [
            ("spellcasting", 6),
            ("counterspelling", 6),
            ("summoning", 6),
            ("assensing", 4),
            ("banishing", 6),
            ("binding", 4),
        ] {
            d.map_set(ACTIVE_SKILLS_FIELD, s, serde_json::json!(r));
        }
        d.map_set(
            KNOWLEDGE_SKILLS_FIELD,
            "Magical Theory",
            serde_json::json!(6),
        );
        d.map_set(KNOWLEDGE_SKILLS_FIELD, "Sprawl Life", serde_json::json!(6));
        d.map_set(LANGUAGE_SKILLS_FIELD, "English", serde_json::json!(4));
        d.map_set(LANGUAGE_SKILLS_FIELD, "Sperethiel", serde_json::json!(4));
        d
    }

    #[test]
    fn below_minimum_metatype_attribute_is_rejected() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "troll");
        // Troll Body/Strength minimum is 5; Body 1 is illegal.
        d.set_number("body", 1);
        d.set_number("strength", 5);
        let pr = priorities('A', 'C', 'E', 'B', 'D');
        let iss = issues(&cfg, &d, &pr);
        assert!(
            iss.iter().any(
                |i| matches!(i, ChargenIssue::Message(m) if m.contains("Body") && m.contains("below"))
            ),
            "expected a below-minimum Body issue, got {iss:?}"
        );
        // finalize refuses it.
        assert!(finalize_sheet(&cfg, &mut d, &pr).is_err());
    }

    #[test]
    fn seed_metatype_minimums_raises_floor_and_never_lowers() {
        let mut d = doc();
        d.set_text("metatype", "troll");
        assert!(seed_metatype_minimums(&mut d));
        assert_eq!(d.number("body"), Some(5));
        assert_eq!(d.number("strength"), Some(5));
        assert_eq!(d.number("charisma"), Some(1)); // troll charisma min 1
        // Never lowers an attribute already above the floor.
        d.set_number("body", 8);
        seed_metatype_minimums(&mut d);
        assert_eq!(d.number("body"), Some(8));
    }

    #[test]
    fn special_attributes_are_capped_at_chargen() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human");
        d.set_text("awakened_type", "magician");
        // Magic 12 fits the special-points ledger (metatype B = 7 ≥ 12−6) but
        // exceeds the chargen Magic cap of 6.
        d.set_number(MAGIC, 12);
        let pr = priorities('B', 'C', 'A', 'D', 'E');
        let iss = issues(&cfg, &d, &pr);
        assert!(
            iss.iter().any(
                |i| matches!(i, ChargenIssue::Message(m) if m.contains("Magic") && m.contains("maximum"))
            ),
            "expected a Magic chargen-cap issue, got {iss:?}"
        );
        // Edge above the metatype maximum is also caught.
        let mut d = doc();
        d.set_text("metatype", "human"); // Edge max 7
        d.set_number(EDGE, 8);
        let iss = issues(&cfg, &d, &priorities('A', 'B', 'E', 'C', 'D'));
        assert!(
            iss.iter().any(
                |i| matches!(i, ChargenIssue::AboveCap { field, cap: 7, .. } if field == EDGE)
            ),
            "expected an Edge AboveCap issue, got {iss:?}"
        );
    }

    #[test]
    fn positive_qualities_cap_independently_at_25() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human");
        // 30 positive + 25 negative would fit karma_total (25 + 25 = 50), but
        // positive qualities are independently capped at 25 Karma.
        d.map_set("positive_qualities", "Lucky", serde_json::json!(30));
        d.map_set("negative_qualities", "Bad Luck", serde_json::json!(25));
        let iss = issues(&cfg, &d, &priorities('A', 'B', 'E', 'C', 'D'));
        assert!(
            iss.iter()
                .any(|i| matches!(i, ChargenIssue::Message(m) if m.contains("Positive qualities"))),
            "expected the positive-quality cap issue, got {iss:?}"
        );
    }

    #[test]
    fn extra_spells_beyond_the_free_count_draw_karma() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human");
        d.set_text("awakened_type", "magician");
        // Magic priority C grants 5 free spells. Twelve spells = 7 extra × 5 =
        // 35 Karma, over the 25-Karma pool → a Karma overspend.
        let spells: Vec<serde_json::Value> = (0..12)
            .map(|i| serde_json::json!(format!("Spell {i}")))
            .collect();
        d.set("spells", serde_json::Value::Array(spells));
        let pr = priorities('B', 'A', 'C', 'D', 'E'); // magic = C
        let iss = issues(&cfg, &d, &pr);
        assert!(
            iss.iter()
                .any(|i| matches!(i, ChargenIssue::Overspent { pool, .. } if pool == "karma")),
            "extra spells must draw Karma into overspend, got {iss:?}"
        );
    }

    #[test]
    fn free_magical_skills_offset_magical_skill_cost() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human");
        d.set_text("awakened_type", "magician");
        // magic = A → two free Magical skills @ Rating 5 = 10 free magical points.
        let pr = priorities('C', 'E', 'A', 'D', 'B');
        d.map_set(ACTIVE_SKILLS_FIELD, "spellcasting", serde_json::json!(6));
        d.map_set(ACTIVE_SKILLS_FIELD, "summoning", serde_json::json!(4));
        let ls = ledgers(&cfg, &d, &pr);
        // 10 magical points fully covered by the free budget → 0 against the pool.
        assert_eq!(ls.iter().find(|l| l.name == "skills").unwrap().spent, 0);
        let mag = ls.iter().find(|l| l.name == "magical_skills").unwrap();
        assert_eq!((mag.total, mag.spent), (10, 10));
    }

    #[test]
    fn contacts_beyond_the_free_pool_draw_karma() {
        let cfg = Sr5Config::default();
        let mut d = doc();
        d.set_text("metatype", "human");
        d.set_number("charisma", 2); // free contact pool = Charisma × 3 = 6
        d.map_set("contacts_connection", "Fixer", serde_json::json!(6));
        d.map_set("contacts_loyalty", "Fixer", serde_json::json!(6));
        let ls = ledgers(&cfg, &d, &priorities('A', 'B', 'E', 'C', 'D'));
        let c = ls.iter().find(|l| l.name == "contacts").unwrap();
        assert_eq!((c.total, c.spent), (6, 12));
        // 12 − 6 free = 6 Karma spent.
        assert_eq!(ls.iter().find(|l| l.name == "karma").unwrap().spent, 6);
    }

    #[test]
    fn mystic_adept_power_points_are_not_free() {
        let cfg = Sr5Config::default();
        let pr = priorities('C', 'E', 'A', 'D', 'B');
        // A pure Adept cannot take magic = A (Adept starts at B), so use the
        // full-mage build for both and compare the Power-Point seeding.
        let mut mystic = balanced_full_mage("mystic_adept");
        assert!(issues(&cfg, &mystic, &pr).is_empty());
        finalize_sheet(&cfg, &mut mystic, &pr).unwrap();
        assert_eq!(mystic.number(MAGIC), Some(6));
        // Mystic adepts buy Power Points with Karma; they are NOT seeded free.
        assert_ne!(mystic.number("power_points"), Some(6));
    }
}
