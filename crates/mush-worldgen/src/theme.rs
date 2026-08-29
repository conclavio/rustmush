//! Theme objects: the `worldgen.theme` JSON attribute, hierarchical
//! resolution, and the merge rules (design 03 §5.1).
//!
//! A [`Theme`] is data serialized into a `worldgen.theme` attribute on any
//! object — a Zone Master Object, a room (room-level override), or Limbo
//! `#0` (the game default). It is a **normal attribute** (no module
//! namespace claim): the `@theme` command writes it through the ordinary
//! attribute path, gated by `can_modify`.

use mush_core::attr::AttrValue;
use mush_core::{LIMBO, ObjId, World};
use serde::{Deserialize, Serialize};

/// The attribute a theme is stored under.
pub const THEME_ATTR: &str = "worldgen.theme";

/// How long generated descriptions should run.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DescLength {
    /// A couple of sentences (maps, glances).
    Short,
    /// A solid paragraph (the default).
    #[default]
    Medium,
    /// A rich, multi-sentence room.
    Long,
}

impl DescLength {
    /// Parse the length from a `@theme` value (`short`/`medium`/`long`).
    #[must_use]
    pub fn parse(s: &str) -> Option<DescLength> {
        match s.trim().to_ascii_lowercase().as_str() {
            "short" => Some(DescLength::Short),
            "medium" | "med" => Some(DescLength::Medium),
            "long" => Some(DescLength::Long),
            _ => None,
        }
    }

    /// The wire string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DescLength::Short => "short",
            DescLength::Medium => "medium",
            DescLength::Long => "long",
        }
    }

    /// A rough output-token target the prompt states to the model.
    #[must_use]
    pub fn token_target(self) -> u32 {
        match self {
            DescLength::Short => 120,
            DescLength::Medium => 250,
            DescLength::Long => 450,
        }
    }
}

/// A worldbuilding theme (design 03 §5.1). All fields optional/empty by
/// default so a partially-specified theme merges cleanly with its ancestors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Human name for the theme (e.g. "1920s Chicago").
    pub name: String,
    /// Free prose: era, mood, architectural palette.
    pub style_guide: String,
    /// Words/motifs to favor.
    pub vocabulary: Vec<String>,
    /// Words/anachronisms to avoid.
    pub banned: Vec<String>,
    /// Gold-standard descs pasted by staff (via `@theme/exemplar`).
    pub exemplars: Vec<String>,
    /// Target description length.
    pub desc_length: DescLength,
    /// POV/tense rules, e.g. "second person, present tense, no 'you see'".
    pub pov_rules: String,
}

impl Theme {
    /// Whether every field is at its default (an absent/empty theme).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
            && self.style_guide.is_empty()
            && self.vocabulary.is_empty()
            && self.banned.is_empty()
            && self.exemplars.is_empty()
            && self.pov_rules.is_empty()
            && self.desc_length == DescLength::default()
    }

    /// Set one field from a `@theme <obj>=<field>:<value>` edit. List fields
    /// are set from a comma-separated value. Returns the canonical field
    /// name on success, or `Err` with the list of valid fields.
    pub fn set_field(&mut self, field: &str, value: &str) -> Result<&'static str, String> {
        let list = |v: &str| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        match field.trim().to_ascii_lowercase().as_str() {
            "name" => {
                self.name = value.trim().to_string();
                Ok("name")
            }
            "style_guide" | "style" => {
                self.style_guide = value.trim().to_string();
                Ok("style_guide")
            }
            "vocabulary" | "vocab" => {
                self.vocabulary = list(value);
                Ok("vocabulary")
            }
            "banned" => {
                self.banned = list(value);
                Ok("banned")
            }
            "exemplars" => {
                self.exemplars = list(value);
                Ok("exemplars")
            }
            "desc_length" | "length" => {
                self.desc_length = DescLength::parse(value)
                    .ok_or_else(|| "desc_length must be short, medium, or long.".to_string())?;
                Ok("desc_length")
            }
            "pov_rules" | "pov" => {
                self.pov_rules = value.trim().to_string();
                Ok("pov_rules")
            }
            other => Err(format!(
                "Unknown theme field {other:?}. Valid: name, style_guide, vocabulary, \
                 banned, exemplars, desc_length, pov_rules."
            )),
        }
    }
}

/// Read the raw theme stored directly on `obj` (no inheritance walk).
#[must_use]
pub fn theme_on(w: &World, obj: ObjId) -> Option<Theme> {
    let attr = w.get(obj)?.attrs.get(THEME_ATTR)?;
    match &attr.value {
        AttrValue::Json(v) => serde_json::from_value(v.clone()).ok(),
        // Tolerate a theme accidentally stored as text JSON.
        AttrValue::Text(s) => serde_json::from_str(s).ok(),
    }
}

/// The object chain a room resolves its theme through: the room itself, its
/// zone, that zone's zone (bounded), then Limbo `#0` (the game default).
/// Nearest first; duplicates and cycles are dropped.
#[must_use]
pub fn theme_chain(w: &World, room: ObjId) -> Vec<ObjId> {
    const MAX_ZONE_HOPS: usize = 8;
    let mut chain = vec![room];
    let mut cur = w.get(room).and_then(|o| o.zone);
    let mut hops = 0;
    while let Some(z) = cur {
        if chain.contains(&z) || hops >= MAX_ZONE_HOPS {
            break;
        }
        chain.push(z);
        cur = w.get(z).and_then(|o| o.zone);
        hops += 1;
    }
    if !chain.contains(&LIMBO) {
        chain.push(LIMBO);
    }
    chain
}

/// Resolve the effective theme for `room` (design 03 §5.1): room override →
/// its zone → parent zone chain → game default (`#0`).
///
/// Merge rules: **scalar** fields (name, style_guide, pov_rules,
/// desc_length) take the nearest non-empty value; **list** fields
/// (vocabulary, banned, exemplars) are unioned across the whole chain,
/// nearest first, de-duplicated — so a room's favored words add to, rather
/// than replace, its zone's.
#[must_use]
pub fn resolve_theme(w: &World, room: ObjId) -> Theme {
    let chain = theme_chain(w, room);
    let themes: Vec<Theme> = chain.iter().filter_map(|&o| theme_on(w, o)).collect();
    let mut out = Theme::default();
    // Scalars: nearest non-empty wins.
    for t in &themes {
        if out.name.is_empty() && !t.name.is_empty() {
            out.name = t.name.clone();
        }
        if out.style_guide.is_empty() && !t.style_guide.is_empty() {
            out.style_guide = t.style_guide.clone();
        }
        if out.pov_rules.is_empty() && !t.pov_rules.is_empty() {
            out.pov_rules = t.pov_rules.clone();
        }
    }
    // desc_length: nearest theme that carries a non-default value; else the
    // nearest theme's value; else the global default.
    //
    // Known limitation: `Medium` is the default, so an *explicit*
    // `desc_length: medium` set on a room is indistinguishable from "unset"
    // and cannot override an ancestor `long`/`short` — the ancestor's
    // non-default value wins. Distinguishing the two would require tracking
    // explicit-set (an `Option<DescLength>`) through the serialized theme;
    // deferred as a data-model change. Builders wanting a shorter room under a
    // `long` zone use `short` (which is non-default and does override).
    if let Some(t) = themes
        .iter()
        .find(|t| t.desc_length != DescLength::default())
        .or_else(|| themes.first())
    {
        out.desc_length = t.desc_length;
    }
    // Lists: union nearest-first, de-duplicated (case-insensitive).
    for t in &themes {
        union_into(&mut out.vocabulary, &t.vocabulary);
        union_into(&mut out.banned, &t.banned);
        union_into(&mut out.exemplars, &t.exemplars);
    }
    out
}

fn union_into(dst: &mut Vec<String>, src: &[String]) {
    for s in src {
        if !dst.iter().any(|e| e.eq_ignore_ascii_case(s)) {
            dst.push(s.clone());
        }
    }
}
