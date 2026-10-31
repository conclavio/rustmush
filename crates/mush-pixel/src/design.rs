//! **The staff unit designer's look half** (doc 31 cut UD.2; D-31-7 as amended
//! on 2026-10-30): the checks a look meets against its unit and its model, and
//! the model preview — both run in the browser, in `mush-pixel-wasm`, while the
//! unit's rules are checked on the server (`tacsim::design`).
//!
//! [`check_look`] is the one implementation of doc 22 §6's look-against-unit
//! rules: `mush-contract-tests`' `unit_looks` holds every shipped look to it,
//! and the designer refuses a draft with the same sentences before export.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::bake::{self, View};
use crate::look::{Facing, Look};
use crate::raycast::SpriteKind;

/// What a unit's rules say about it, as far as a look needs them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitShape {
    /// Each section's id and its facets' ids, in the unit's order.
    pub sections: Vec<(String, Vec<String>)>,
    /// Its weapon component ids.
    pub weapons: Vec<String>,
}

impl UnitShape {
    /// Read a unit's `def` object (the rules pack entry's `def`): its
    /// `sections[].id` and `facets[].id`, and the ids of its components whose
    /// `role.kind` is `weapon`.
    #[must_use]
    pub fn of_def(def: &Value) -> UnitShape {
        let id = |v: &Value| v.get("id").and_then(Value::as_str).map(str::to_string);
        let sections = def
            .get("sections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let facets = s
                    .get("facets")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(id)
                    .collect();
                Some((id(s)?, facets))
            })
            .collect();
        let weapons = def
            .get("components")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|c| c.pointer("/role/kind").and_then(Value::as_str) == Some("weapon"))
            .filter_map(id)
            .collect();
        UnitShape { sections, weapons }
    }

    fn facets(&self, section: &str) -> Option<&[String]> {
        self.sections
            .iter()
            .find(|(s, _)| s == section)
            .map(|(_, f)| f.as_slice())
    }
}

/// What a look check found: refusals (the look does not fit) and notes (it
/// fits, with something worth a second look).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookCheck {
    /// Each reason the look does not fit its unit or its model.
    pub refusals: Vec<String>,
    /// Things that hold but are worth saying: a weapon the model shows no
    /// muzzle for (doc 31 D-31-5).
    pub notes: Vec<String>,
}

/// **Check `look` against the units it draws and the model it names** (doc 22
/// §6, doc 31 D-31-1 and D-31-5): every section of every unit is in its doll;
/// every doll part names a section — and a facet of it — each unit has; every
/// weapon short name, and every muzzle of the sheet it draws, names a weapon
/// component of one of them; its model is in the catalogue, its `size_cm`
/// within the matrix check's bounds of the model's (height within 2 %, width
/// within half to twice). A weapon of the unit the model shows no muzzle for is
/// a note, not a refusal: a hull-internal launcher has no barrel to see.
#[must_use]
pub fn check_look(look: &Look, units: &[(&str, &UnitShape)]) -> LookCheck {
    let mut out = LookCheck::default();
    let name = if look.label.is_empty() {
        look.id.as_str()
    } else {
        look.label.as_str()
    };
    for (unit, shape) in units {
        if let Some(doll) = &look.doll {
            let drawn = doll.sections();
            for (section, _) in &shape.sections {
                if !drawn.contains(&section.as_str()) {
                    out.refusals
                        .push(format!("{unit}: section {section} is not in {name}'s doll"));
                }
            }
            for part in &doll.parts {
                match shape.facets(&part.section) {
                    None => out.refusals.push(format!(
                        "{unit} has no section {} ({name}'s doll)",
                        part.section
                    )),
                    Some(facets) => {
                        if let Some(facet) = &part.facet
                            && !facets.contains(facet)
                        {
                            out.refusals.push(format!(
                                "{unit}: section {} has no facet {facet} ({name}'s doll)",
                                part.section
                            ));
                        }
                    }
                }
            }
        }
    }
    let muzzles: Vec<&str> = look
        .sheet()
        .into_iter()
        .flat_map(|sheet| sheet.facings.values())
        .flat_map(|facing| facing.muzzles.keys().map(String::as_str))
        .collect();
    let fitted = |weapon: &str| {
        units
            .iter()
            .any(|(_, s)| s.weapons.iter().any(|w| w == weapon))
    };
    let mut said: Vec<&str> = Vec::new();
    for weapon in look
        .weapons
        .keys()
        .map(String::as_str)
        .chain(muzzles.iter().copied())
    {
        if !fitted(weapon) && !said.contains(&weapon) {
            said.push(weapon);
            out.refusals.push(format!(
                "{name}: weapon {weapon} is fitted to none of its units"
            ));
        }
    }
    if let Some(model) = look.model.as_deref() {
        match crate::model::entry(model) {
            None => out
                .refusals
                .push(format!("{name}: model {model} is not in the catalogue")),
            Some(entry) => {
                let ((lw, lh), (mw, mh)) = (look.size_cm, entry.size_cm);
                if mh.abs_diff(lh) * 50 > lh {
                    out.refusals
                        .push(format!("{name}: {lh} cm tall, its model {model} {mh} cm"));
                }
                if mw * 2 < lw || mw > lw * 2 {
                    out.refusals
                        .push(format!("{name}: {lw} cm wide, its model {model} {mw} cm"));
                }
                let mut seen: Vec<&str> = Vec::new();
                for (_, shape) in units {
                    for weapon in &shape.weapons {
                        if !muzzles.contains(&weapon.as_str()) && !seen.contains(&weapon.as_str()) {
                            seen.push(weapon);
                            out.notes
                                .push(format!("{name}: the model shows no muzzle for {weapon}"));
                        }
                    }
                }
            }
        }
    }
    out
}

/// **Check a draft look's JSON against a draft unit's `def`** — what the
/// designer runs on every edit: the look must parse ([`Look::parse`]), then
/// [`check_look`] against the one unit.
#[must_use]
pub fn check_draft(look_json: &str, def: &Value) -> LookCheck {
    let look = match Look::parse(look_json) {
        Ok(look) => look,
        Err(e) => {
            return LookCheck {
                refusals: vec![e.0],
                notes: Vec::new(),
            };
        }
    };
    let shape = UnitShape::of_def(def);
    let unit = look.units.first().map_or("the unit", String::as_str);
    check_look(&look, &[(unit, &shape)])
}

/// **A model's frame as RGBA** for the designer's preview: model `id`'s
/// `view` chain at the size nearest above `size_px`, `facing`, pose `pose`
/// (`0` still, `1..=4` moving, `5` wreck), painted as `kind` would draw it.
/// The model's bake when it is installed, else its fallback sheet as a chain
/// at its catalogue `size_cm`. With the frame's side, pixels.
#[must_use]
pub fn preview_rgba(
    id: &str,
    view: View,
    size_px: u16,
    facing: Facing,
    pose: usize,
    kind: SpriteKind,
) -> Option<(usize, Vec<u8>)> {
    static SHEET_CHAINS: std::sync::Mutex<BTreeMap<(String, View), &'static bake::Chain>> =
        std::sync::Mutex::new(BTreeMap::new());
    let chain: &bake::Chain = match bake::model(id).and_then(|m| m.chain(view)) {
        Some(chain) => chain,
        None => {
            let mut cache = SHEET_CHAINS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(chain) = cache.get(&(id.to_string(), view)) {
                chain
            } else {
                let sheet = crate::model::sheet(id)?;
                let size_cm = crate::model::entry(id)?.size_cm;
                let chain: &'static bake::Chain =
                    Box::leak(Box::new(bake::chain_from_sheet(sheet, size_cm, view)));
                cache.insert((id.to_string(), view), chain);
                chain
            }
        }
    };
    let sized = chain
        .sizes
        .iter()
        .filter(|s| s.size >= size_px)
        .min_by_key(|s| s.size)
        .or_else(|| chain.sizes.iter().max_by_key(|s| s.size))?;
    let frame = sized.frame(facing, pose)?;
    let n = usize::from(sized.size);
    let mut rgba = vec![0u8; n * n * 4];
    let kind = if pose == bake::POSE_WRECK {
        SpriteKind::Wreck
    } else {
        kind
    };
    for y in 0..n {
        for x in 0..n {
            #[allow(clippy::cast_possible_wrap)]
            let code = frame.code(x as i64, y as i64);
            if let Some(material) = bake::material_from_code(code) {
                let [r, g, b] = crate::iso25d::material_rgb(material, kind);
                let i = (y * n + x) * 4;
                if let Some(px) = rgba.get_mut(i..i + 4) {
                    px.copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
    }
    Some((n, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped line tank's `def`.
    fn tank_def() -> Value {
        let pack: Value =
            serde_json::from_str(include_str!("../../tacsim/content/units/line_tank.json"))
                .expect("JSON");
        pack["entries"]["tacsim.unit.line_tank"]["def"].clone()
    }

    fn tank_look() -> String {
        crate::look::shipped_sources()
            .iter()
            .find(|(n, _)| *n == "tracked_tank.json")
            .map(|(_, t)| (*t).to_string())
            .expect("the tank's look")
    }

    #[test]
    fn the_shipped_tank_look_fits_a_tank() {
        let found = check_draft(&tank_look(), &tank_def());
        // The look names the laser too, which this tank does not fit.
        assert_eq!(
            found.refusals,
            vec!["Tracked tank: weapon laser is fitted to none of its units".to_string()]
        );
    }

    #[test]
    fn a_look_that_does_not_fit_is_refused_with_a_reason() {
        let mut def = tank_def();
        def["sections"]
            .as_array_mut()
            .expect("sections")
            .push(serde_json::json!({ "id": "sponson", "facets": [] }));
        let found = check_draft(&tank_look(), &def);
        assert!(
            found
                .refusals
                .iter()
                .any(|r| r.contains("sponson is not in"))
        );
        let mut look: Value = serde_json::from_str(&tank_look()).expect("JSON");
        look["size_cm"] = serde_json::json!([700, 900]);
        look["model"] = "mc.st.tank_lvl2".into();
        let found = check_draft(&look.to_string(), &tank_def());
        assert!(
            found.refusals.iter().any(|r| r.contains("cm tall")),
            "{found:?}"
        );
        look["model"] = "mc.nobody".into();
        let found = check_draft(&look.to_string(), &tank_def());
        assert!(
            found
                .refusals
                .iter()
                .any(|r| r.contains("not in the catalogue"))
        );
        let found = check_draft("{}", &tank_def());
        assert_eq!(
            found.refusals.len(),
            1,
            "a look that will not parse says why"
        );
    }

    #[test]
    fn every_model_previews_at_every_view() {
        for m in crate::model::index() {
            for view in View::ALL {
                let (n, rgba) =
                    preview_rgba(&m.id, view, 96, Facing::FrontRight, 0, SpriteKind::Hostile)
                        .unwrap_or_else(|| panic!("{} {}", m.id, view.name()));
                assert_eq!(n, 96);
                assert!(rgba.chunks(4).any(|px| px[3] == 255), "{}: drawn", m.id);
            }
        }
        assert!(
            preview_rgba(
                "mc.nobody",
                View::Fp,
                96,
                Facing::Front,
                0,
                SpriteKind::Hostile
            )
            .is_none()
        );
    }
}
