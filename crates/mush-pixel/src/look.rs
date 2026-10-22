//! **A unit's look** (doc 22, the unit kit): how a unit type is drawn, kept
//! apart from its rules (D-22-1) — the rules pack never carries art and art
//! never moves a rules digest. A look names the unit types it draws and
//! carries:
//!
//! - `label` and `short`: what a screen prints for the type;
//! - `weapons`: a short name a weapon component, for the text panel;
//! - `doll`: the grid its damage is painted onto (D-22-2), at most
//!   [`DOLL_MAX_W`] by [`DOLL_MAX_H`] cells written as rows of characters, a
//!   legend naming the section (and optionally the facet) each character
//!   stands for, `.` empty; `view` is `top` (a vehicle from above, front up)
//!   or `upright` (a walker standing, its left on the left, as its pilot
//!   thinks of it);
//! - `sprites`: the sprite sheet (doc 22 §3.2; cut U.3).
//!
//! The shipped looks are JSON files under `crates/mush-pixel/looks/`, embedded
//! here ([`shipped`]); [`Look::parse`] is the check a look must pass on its own
//! (D-22-6) — the check against its unit's rules is a contract test, which
//! sees both halves.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

/// The widest a doll may be, cells.
pub const DOLL_MAX_W: usize = 24;
/// The tallest a doll may be, cells.
pub const DOLL_MAX_H: usize = 16;

/// The looks this build ships, by file name and text.
const SHIPPED: &[(&str, &str)] = &[
    (
        "tracked_tank.json",
        include_str!("../looks/tracked_tank.json"),
    ),
    (
        "wheeled_scout.json",
        include_str!("../looks/wheeled_scout.json"),
    ),
    ("biped.json", include_str!("../looks/biped.json")),
    ("drone.json", include_str!("../looks/drone.json")),
];

/// How a doll is seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DollView {
    /// A vehicle from above, front up.
    Top,
    /// A walker standing, its left on the left.
    Upright,
}

/// What one doll character stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DollPart {
    /// The section.
    pub section: String,
    /// The facet of it, where the cell shows one face's armour.
    pub facet: Option<String>,
}

/// The doll: a grid of cells, each empty or a part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doll {
    /// How it is seen.
    pub view: DollView,
    /// Cells across.
    pub width: usize,
    /// Cells down.
    pub height: usize,
    /// The parts, indexed by [`Doll::cells`].
    pub parts: Vec<DollPart>,
    /// Row by row from the top: the index of the cell's part, or `None`.
    pub cells: Vec<Option<usize>>,
}

impl Doll {
    /// The part at `(x, y)`, if any.
    #[must_use]
    pub fn at(&self, x: usize, y: usize) -> Option<&DollPart> {
        if x >= self.width {
            return None;
        }
        let i = (*self.cells.get(y * self.width + x)?)?;
        self.parts.get(i)
    }

    /// Every section the doll draws, once each, in the order first met.
    #[must_use]
    pub fn sections(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for part in &self.parts {
            if !out.contains(&part.section.as_str()) {
                out.push(&part.section);
            }
        }
        out
    }

    /// **A tone a part**, `0` or `1`, so that parts which touch are told apart
    /// when they are in the same state: a renderer draws tone `1` a lighter
    /// shade of the same colour. Chosen greedily in part order — each part
    /// takes the tone none of its already-toned neighbours has, or `0` when
    /// both are taken.
    #[must_use]
    pub fn tones(&self) -> Vec<u8> {
        let n = self.parts.len();
        let mut touches = vec![Vec::<usize>::new(); n];
        for y in 0..self.height {
            for x in 0..self.width {
                let Some(Some(a)) = self.cells.get(y * self.width + x).copied() else {
                    continue;
                };
                let right = (x + 1 < self.width)
                    .then(|| self.cells.get(y * self.width + x + 1).copied().flatten())
                    .flatten();
                let down = self.cells.get((y + 1) * self.width + x).copied().flatten();
                for b in [right, down].into_iter().flatten() {
                    if a != b {
                        if let Some(t) = touches.get_mut(a) {
                            t.push(b);
                        }
                        if let Some(t) = touches.get_mut(b) {
                            t.push(a);
                        }
                    }
                }
            }
        }
        let mut tone: Vec<Option<u8>> = vec![None; n];
        for part in 0..n {
            let taken: Vec<u8> = touches
                .get(part)
                .into_iter()
                .flatten()
                .filter_map(|b| tone.get(*b).copied().flatten())
                .collect();
            let pick = if taken.contains(&0) && !taken.contains(&1) {
                1
            } else {
                0
            };
            if let Some(t) = tone.get_mut(part) {
                *t = Some(pick);
            }
        }
        tone.into_iter().map(Option::unwrap_or_default).collect()
    }

    /// What is left of each part, `(left, max)`, read off an `arena.cockpit`
    /// frame's `sections`: a facet's armour where the part names a facet the
    /// section has, else the section's internals; a destroyed section has
    /// none left; a part whose section the frame lacks is `None`.
    #[must_use]
    pub fn condition(&self, sections: &Value) -> Vec<Option<(i64, i64)>> {
        let rows = sections.as_array().map(Vec::as_slice).unwrap_or_default();
        self.parts
            .iter()
            .map(|part| {
                let sec = rows
                    .iter()
                    .find(|s| s.get("id").and_then(Value::as_str) == Some(part.section.as_str()))?;
                let num = |v: Option<&Value>| v.and_then(Value::as_i64).unwrap_or(0);
                let internals = || {
                    let max = num(sec.get("internals_max"));
                    if sec.get("condition").and_then(Value::as_str) == Some("destroyed") {
                        (0, max)
                    } else {
                        (num(sec.get("internals")), max)
                    }
                };
                let facet = part.facet.as_deref().and_then(|id| {
                    sec.get("facets")?
                        .as_array()?
                        .iter()
                        .find(|f| f.get("id").and_then(Value::as_str) == Some(id))
                });
                Some(facet.map_or_else(internals, |f| {
                    (num(f.get("armor")), num(f.get("armor_max")))
                }))
            })
            .collect()
    }
}

/// One unit type's look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    /// The unit types (definition ids) it draws.
    pub units: Vec<String>,
    /// The type's name.
    pub label: String,
    /// Its short name.
    pub short: String,
    /// A short name a weapon component id.
    pub weapons: BTreeMap<String, String>,
    /// The doll, where it has one.
    pub doll: Option<Doll>,
}

/// Why a look was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookError(pub String);

impl std::fmt::Display for LookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(s: impl Into<String>) -> Result<T, LookError> {
    Err(LookError(s.into()))
}

/// The keys an object may carry; any other is refused.
fn only(obj: &serde_json::Map<String, Value>, keys: &[&str], what: &str) -> Result<(), LookError> {
    match obj.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => err(format!("{what}: unknown key `{k}`")),
        None => Ok(()),
    }
}

fn string(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    what: &str,
) -> Result<String, LookError> {
    match obj.get(key).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => err(format!("{what}: `{key}` must be a non-empty string")),
    }
}

impl Look {
    /// Read and check one look (D-22-6): every key known, at least one unit,
    /// a label and a short name; the doll within [`DOLL_MAX_W`] by
    /// [`DOLL_MAX_H`], its rows all one width, every character in the legend
    /// (or `.`), every legend character used, at least one part. The check
    /// against the unit's own sections is the contract test's.
    ///
    /// # Errors
    ///
    /// [`LookError`] naming the first thing that does not hold.
    pub fn parse(text: &str) -> Result<Look, LookError> {
        let v: Value =
            serde_json::from_str(text).map_err(|e| LookError(format!("not JSON: {e}")))?;
        let Some(obj) = v.as_object() else {
            return err("a look is an object");
        };
        only(
            obj,
            &["units", "label", "short", "weapons", "doll", "sprites"],
            "look",
        )?;
        let units: Vec<String> = match obj.get("units").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list
                .iter()
                .map(|u| u.as_str().map(str::to_string))
                .collect::<Option<_>>()
                .ok_or_else(|| LookError("look: `units` must be strings".into()))?,
            _ => return err("look: `units` must name at least one unit type"),
        };
        let label = string(obj, "label", "look")?;
        let short = string(obj, "short", "look")?;
        let mut weapons = BTreeMap::new();
        if let Some(w) = obj.get("weapons") {
            let Some(w) = w.as_object() else {
                return err("look: `weapons` must be an object");
            };
            for (id, name) in w {
                match name.as_str() {
                    Some(n) if !n.is_empty() => {
                        weapons.insert(id.clone(), n.to_string());
                    }
                    _ => return err(format!("look: weapon `{id}` needs a short name")),
                }
            }
        }
        let doll = obj.get("doll").map(parse_doll).transpose()?;
        Ok(Look {
            units,
            label,
            short,
            weapons,
            doll,
        })
    }

    /// The short name of weapon component `id`: the look's, else the id.
    #[must_use]
    pub fn weapon_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.weapons.get(id).map_or(id, String::as_str)
    }
}

fn parse_doll(v: &Value) -> Result<Doll, LookError> {
    let Some(obj) = v.as_object() else {
        return err("doll: must be an object");
    };
    only(obj, &["view", "rows", "legend"], "doll")?;
    let view = match obj.get("view").and_then(Value::as_str) {
        Some("top") => DollView::Top,
        Some("upright") => DollView::Upright,
        _ => return err("doll: `view` is `top` or `upright`"),
    };
    let rows: Vec<&str> = match obj.get("rows").and_then(Value::as_array) {
        Some(r) if !r.is_empty() => r
            .iter()
            .map(Value::as_str)
            .collect::<Option<_>>()
            .ok_or_else(|| LookError("doll: rows must be strings".into()))?,
        _ => return err("doll: `rows` must be a non-empty list"),
    };
    let width = rows.first().map_or(0, |r| r.chars().count());
    if width == 0 || width > DOLL_MAX_W || rows.len() > DOLL_MAX_H {
        return err(format!(
            "doll: {}x{} cells; at most {DOLL_MAX_W}x{DOLL_MAX_H}",
            width,
            rows.len()
        ));
    }
    if let Some((i, _)) = rows
        .iter()
        .enumerate()
        .find(|(_, r)| r.chars().count() != width)
    {
        return err(format!("doll: row {i} is not {width} cells wide"));
    }
    let Some(legend) = obj.get("legend").and_then(Value::as_object) else {
        return err("doll: `legend` must be an object");
    };
    let mut keys: Vec<char> = Vec::new();
    let mut parts: Vec<DollPart> = Vec::new();
    for (key, part) in legend {
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return err(format!("doll: legend key `{key}` must be one character"));
        };
        if c == '.' || c.is_whitespace() {
            return err("doll: `.` and spaces are empty cells, not legend keys");
        }
        let Some(p) = part.as_object() else {
            return err(format!("doll: legend `{key}` must be an object"));
        };
        only(p, &["section", "facet"], "doll legend")?;
        let section = string(p, "section", "doll legend")?;
        let facet = match p.get("facet") {
            None => None,
            Some(_) => Some(string(p, "facet", "doll legend")?),
        };
        keys.push(c);
        parts.push(DollPart { section, facet });
    }
    let mut used = vec![false; keys.len()];
    let mut cells = Vec::with_capacity(width * rows.len());
    for (y, row) in rows.iter().enumerate() {
        for c in row.chars() {
            if c == '.' || c == ' ' {
                cells.push(None);
                continue;
            }
            match keys.iter().position(|k| *k == c) {
                Some(i) => {
                    if let Some(u) = used.get_mut(i) {
                        *u = true;
                    }
                    cells.push(Some(i));
                }
                None => return err(format!("doll: row {y} uses `{c}`, which the legend lacks")),
            }
        }
    }
    if let Some(i) = used.iter().position(|u| !u) {
        let key = keys.get(i).copied().unwrap_or('?');
        return err(format!("doll: legend `{key}` is never drawn"));
    }
    Ok(Doll {
        view,
        width,
        height: rows.len(),
        parts,
        cells,
    })
}

/// The looks this build ships, parsed once. A shipped look that does not
/// parse is a build defect the tests catch; it is left out here rather than
/// taking a renderer down.
#[must_use]
pub fn shipped() -> &'static [Look] {
    static LOOKS: OnceLock<Vec<Look>> = OnceLock::new();
    LOOKS.get_or_init(|| {
        SHIPPED
            .iter()
            .filter_map(|(_, text)| Look::parse(text).ok())
            .collect()
    })
}

/// The shipped look that draws unit type `unit`, if any.
#[must_use]
pub fn for_unit(unit: &str) -> Option<&'static Look> {
    shipped().iter().find(|l| l.units.iter().any(|u| u == unit))
}

/// The shipped looks' file names and texts, for the contract test.
#[must_use]
pub fn shipped_sources() -> &'static [(&'static str, &'static str)] {
    SHIPPED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_look_parses() {
        for (name, text) in SHIPPED {
            if let Err(e) = Look::parse(text) {
                panic!("{name}: {e}");
            }
        }
        assert_eq!(shipped().len(), SHIPPED.len());
        assert!(for_unit("tacsim.unit.line_tank").is_some());
        assert!(for_unit("tacsim.unit.biped_mk1").is_some());
        assert!(for_unit("tacsim.unit.nonesuch").is_none());
    }

    /// The check refuses what doc 22 §6 says it must, with a reason.
    #[test]
    fn the_check_refuses_with_a_reason() {
        let base =
            |doll: &str| format!(r#"{{"units":["u"],"label":"U","short":"U","doll":{doll}}}"#);
        let refused = |text: String, why: &str| {
            let e = Look::parse(&text).expect_err(why);
            assert!(e.0.contains(why), "{e} should say {why}");
        };
        refused(
            base(r#"{"view":"top","rows":["aa","a"],"legend":{"a":{"section":"s"}}}"#),
            "row 1",
        );
        refused(
            base(r#"{"view":"top","rows":["ab"],"legend":{"a":{"section":"s"}}}"#),
            "lacks",
        );
        refused(
            base(
                r#"{"view":"top","rows":["aa"],"legend":{"a":{"section":"s"},"b":{"section":"t"}}}"#,
            ),
            "never drawn",
        );
        refused(
            base(&format!(
                r#"{{"view":"top","rows":["{}"],"legend":{{"a":{{"section":"s"}}}}}}"#,
                "a".repeat(25)
            )),
            "at most",
        );
        refused(
            base(r#"{"view":"side","rows":["a"],"legend":{"a":{"section":"s"}}}"#),
            "view",
        );
        refused(
            r#"{"units":["u"],"label":"U","short":"U","colour":"red"}"#.to_string(),
            "unknown key",
        );
        refused(
            r#"{"units":[],"label":"U","short":"U"}"#.to_string(),
            "at least one",
        );
    }

    /// A part is coloured by its facet's armour where it names one the
    /// section has, else by the section's internals; a destroyed section has
    /// nothing left.
    #[test]
    fn the_condition_reads_facets_then_internals() {
        let look = for_unit("tacsim.unit.line_tank").expect("the tank's look");
        let doll = look.doll.as_ref().expect("a doll");
        let cockpit: Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("fixture");
        let cond = doll.condition(&cockpit["sections"]);
        let of = |section: &str, facet: Option<&str>| {
            let i = doll
                .parts
                .iter()
                .position(|p| p.section == section && p.facet.as_deref() == facet)
                .expect("the part");
            cond[i]
        };
        assert_eq!(of("hull", Some("front")), Some((55, 80)));
        assert_eq!(
            of("hull", Some("left")),
            Some((60, 60)),
            "no left facet: internals"
        );
        assert_eq!(of("right_track", None), Some((0, 24)), "destroyed");
        assert_eq!(of("left_track", None), Some((6, 24)));
    }
}
