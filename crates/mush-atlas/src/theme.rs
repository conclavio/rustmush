//! Themes on the atlas (S2b, plan §3.3; doc 14 D-14-28): the worldbuilding
//! [`Theme`] that `mush-worldgen` prompts with, hung off places instead of zone
//! master objects. A room resolves its theme through its site, its neighborhood,
//! the locality, the region and then the **world theme** (`mod:atlas:theme:world`),
//! with design 03 §5.1's merge rules kept whole: the nearest scalar wins, list
//! fields union nearest-first. The `worldgen.theme` attribute on a zone or a room is
//! no longer read; the one on `#0` still is, read-only, as the world default a
//! world set before the atlas existed.

use mush_core::attr::AttrValue;
use mush_core::{LIMBO, ObjId, World};
use serde::{Deserialize, Serialize};

use crate::module::AtlasState;
use crate::place::{PlaceRef, chain_for_place, chain_for_room};
use crate::record::Records;

/// The world theme document's key (`mod:atlas:theme:world`).
pub const WORLD_THEME_KEY: &str = "theme:world";
/// The attribute a theme used to be stored under; read on `#0` only, as the
/// pre-atlas world default.
pub const LEGACY_THEME_ATTR: &str = "worldgen.theme";
/// The most bytes a prose field holds.
pub const MAX_THEME_TEXT_BYTES: usize = 4 * 1024;
/// The most entries a list field holds.
pub const MAX_THEME_LIST: usize = 64;
/// The most bytes one list entry holds.
pub const MAX_THEME_ITEM_BYTES: usize = 256;
/// The most exemplars a theme keeps.
pub const MAX_EXEMPLARS: usize = 16;
/// The most bytes one exemplar holds.
pub const MAX_EXEMPLAR_BYTES: usize = 4 * 1024;

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
    /// Parse the length from a theme value (`short`/`medium`/`long`).
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

/// A worldbuilding theme (design 03 §5.1). All fields optional/empty by default so
/// a partially-specified theme merges cleanly with the places above it.
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
    /// Gold-standard descs pasted by staff (via `@atlas/theme/exemplar`).
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

    /// Set one field from a `<field>:<value>` edit. List fields are set from a
    /// comma-separated value. Returns the canonical field name on success, or `Err`
    /// with the list of valid fields.
    ///
    /// # Errors
    ///
    /// An unknown field, or a `desc_length` that is not short, medium or long.
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

    /// Re-check every bound a stored theme must hold.
    ///
    /// # Errors
    ///
    /// A message naming the field.
    pub fn validate(&self) -> Result<(), String> {
        let text = |field: &str, s: &str| -> Result<(), String> {
            if s.len() > MAX_THEME_TEXT_BYTES {
                return Err(format!(
                    "theme.{field} is over {MAX_THEME_TEXT_BYTES} bytes"
                ));
            }
            Ok(())
        };
        let list = |field: &str, v: &[String], item_max: usize, max: usize| -> Result<(), String> {
            if v.len() > max {
                return Err(format!("theme.{field} holds more than {max} entries"));
            }
            if v.iter().any(|s| s.len() > item_max) {
                return Err(format!("a theme.{field} entry is over {item_max} bytes"));
            }
            Ok(())
        };
        text("name", &self.name)?;
        text("style_guide", &self.style_guide)?;
        text("pov_rules", &self.pov_rules)?;
        list(
            "vocabulary",
            &self.vocabulary,
            MAX_THEME_ITEM_BYTES,
            MAX_THEME_LIST,
        )?;
        list("banned", &self.banned, MAX_THEME_ITEM_BYTES, MAX_THEME_LIST)?;
        list(
            "exemplars",
            &self.exemplars,
            MAX_EXEMPLAR_BYTES,
            MAX_EXEMPLARS,
        )?;
        Ok(())
    }
}

/// The world theme document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldThemeDoc {
    /// The theme.
    pub theme: Theme,
}

/// Merge a chain of themes, nearest first (design 03 §5.1): **scalar** fields take
/// the nearest non-empty value; `desc_length` the nearest non-default value, else
/// the nearest theme's; **list** fields union across the chain, nearest first,
/// de-duplicated case-insensitively.
#[must_use]
pub fn merge<'a>(themes: impl IntoIterator<Item = &'a Theme>) -> Theme {
    let themes: Vec<&Theme> = themes.into_iter().collect();
    let mut out = Theme::default();
    for t in &themes {
        if out.name.is_empty() && !t.name.is_empty() {
            out.name.clone_from(&t.name);
        }
        if out.style_guide.is_empty() && !t.style_guide.is_empty() {
            out.style_guide.clone_from(&t.style_guide);
        }
        if out.pov_rules.is_empty() && !t.pov_rules.is_empty() {
            out.pov_rules.clone_from(&t.pov_rules);
        }
    }
    // Known limitation, kept from the zone-keyed model: `Medium` is the default, so
    // an explicit `medium` cannot override an ancestor's `long`; use `short`.
    if let Some(t) = themes
        .iter()
        .find(|t| t.desc_length != DescLength::default())
        .or_else(|| themes.first())
    {
        out.desc_length = t.desc_length;
    }
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

/// The theme stored on `#0` under the old attribute, if any: the pre-atlas world
/// default, read and never written here.
#[must_use]
pub fn legacy_world_theme(w: &World) -> Option<Theme> {
    let attr = w.get(LIMBO)?.attrs.get(LEGACY_THEME_ATTR)?;
    match &attr.value {
        AttrValue::Json(v) => serde_json::from_value(v.clone()).ok(),
        AttrValue::Text(s) => serde_json::from_str(s).ok(),
    }
}

/// The world theme: the atlas's document, else the legacy `#0` attribute.
#[must_use]
pub fn world_theme(w: &World) -> Option<Theme> {
    w.module_state::<AtlasState>()
        .and_then(|st| st.world_theme.clone())
        .or_else(|| legacy_world_theme(w))
}

/// The theme a place carries itself (no chain).
#[must_use]
pub fn theme_on(records: &Records, world: Option<&Theme>, place: &PlaceRef) -> Option<Theme> {
    match place {
        PlaceRef::World => world.cloned(),
        PlaceRef::Region(k) => records.regions.get(k).and_then(|r| r.theme.clone()),
        PlaceRef::Locality(k) => records.localities.get(k).and_then(|r| r.theme.clone()),
        PlaceRef::Neighborhood(k) => records.neighborhoods.get(k).and_then(|r| r.theme.clone()),
        PlaceRef::Site(k) => records.sites.get(k).and_then(|r| r.theme.clone()),
    }
}

fn resolve_chain(w: &World, chain: &[PlaceRef]) -> Theme {
    let world = world_theme(w);
    let empty = Records::default();
    let records = crate::place::records(w).unwrap_or(&empty);
    let themes: Vec<Theme> = chain
        .iter()
        .filter_map(|p| theme_on(records, world.as_ref(), p))
        .collect();
    merge(themes.iter())
}

/// Resolve the effective theme for `room`: its site → neighborhood → locality →
/// region → the world. A room off the map resolves to the world theme.
#[must_use]
pub fn resolve_theme(w: &World, room: ObjId) -> Theme {
    let empty = Records::default();
    let records = crate::place::records(w).unwrap_or(&empty);
    let chain = chain_for_room(records, room);
    resolve_chain(w, &chain)
}

/// Resolve the effective theme at a place: the place, then the tiers above it.
#[must_use]
pub fn resolve_place_theme(w: &World, place: &PlaceRef) -> Theme {
    let empty = Records::default();
    let records = crate::place::records(w).unwrap_or(&empty);
    let chain = chain_for_place(records, place);
    resolve_chain(w, &chain)
}

/// The lines `@atlas/theme/show` prints for a resolved theme.
#[must_use]
pub fn render_theme_lines(theme: &Theme) -> Vec<String> {
    if theme.is_empty() {
        return vec!["  (no theme configured)".to_string()];
    }
    let mut lines = Vec::new();
    if !theme.name.is_empty() {
        lines.push(format!("  Name: {}", theme.name));
    }
    if !theme.style_guide.is_empty() {
        lines.push(format!("  Style: {}", theme.style_guide));
    }
    if !theme.pov_rules.is_empty() {
        lines.push(format!("  POV/tense: {}", theme.pov_rules));
    }
    lines.push(format!("  Length: {}", theme.desc_length.as_str()));
    if !theme.vocabulary.is_empty() {
        lines.push(format!("  Favor: {}", theme.vocabulary.join(", ")));
    }
    if !theme.banned.is_empty() {
        lines.push(format!("  Never: {}", theme.banned.join(", ")));
    }
    if !theme.exemplars.is_empty() {
        lines.push(format!("  Exemplars: {}", theme.exemplars.len()));
        for (i, ex) in theme.exemplars.iter().enumerate() {
            let short: String = ex.chars().take(96).collect();
            let more = if ex.chars().count() > 96 { "…" } else { "" };
            lines.push(format!("    {}. {short}{more}", i + 1));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_takes_the_nearest_scalar_and_unions_lists() {
        let near = Theme {
            style_guide: "near".into(),
            vocabulary: vec!["fog".into()],
            ..Theme::default()
        };
        let far = Theme {
            name: "Far".into(),
            style_guide: "far".into(),
            vocabulary: vec!["Fog".into(), "brass".into()],
            desc_length: DescLength::Long,
            ..Theme::default()
        };
        let merged = merge([&near, &far]);
        assert_eq!(merged.style_guide, "near");
        assert_eq!(merged.name, "Far");
        assert_eq!(
            merged.vocabulary,
            vec!["fog".to_string(), "brass".to_string()]
        );
        assert_eq!(merged.desc_length, DescLength::Long);
    }

    #[test]
    fn set_field_knows_its_fields() {
        let mut t = Theme::default();
        assert_eq!(t.set_field("style", "x"), Ok("style_guide"));
        assert_eq!(t.set_field("vocab", "a, b"), Ok("vocabulary"));
        assert_eq!(t.vocabulary, vec!["a".to_string(), "b".to_string()]);
        assert!(t.set_field("colour", "x").is_err());
        assert!(t.set_field("length", "huge").is_err());
    }

    #[test]
    fn validate_bounds_the_lists() {
        let t = Theme {
            vocabulary: (0..=MAX_THEME_LIST).map(|i| i.to_string()).collect(),
            ..Theme::default()
        };
        assert!(t.validate().is_err());
    }
}
