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
//! - `sprites`: the sprite sheet (doc 22 §3.2; cut U.3) — pixel art written
//!   as text, each character a [`Material`] or `.` for nothing: the facings
//!   `front`, `rear` and `right` (and `left`, else `right` mirrored), each a
//!   `still` frame and two to four `moving` frames, one or more `wreck`
//!   frames, the frame's size in pixels, the unit's size in the world, the
//!   pixel that stands on the ground, a muzzle point a weapon a facing.
//!
//! The reserved unit id [`UNKNOWN`] names the look a contact not yet
//! classified is drawn with.
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
/// The largest sprite frame, pixels a side.
pub const SPRITE_MAX: usize = 64;
/// The reserved unit id of the look an unclassified contact is drawn with.
pub const UNKNOWN: &str = "unknown";

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
    ("unknown.json", include_str!("../looks/unknown.json")),
];

/// What a sprite pixel is made of (doc 22 D-22-7): the three team slots,
/// recoloured by the contact's team when it is drawn, and three fixed
/// materials. A closed set, so the picture's palette stays closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// The team's dark shade.
    TeamDark,
    /// The team's colour.
    Team,
    /// The team's light shade.
    TeamLight,
    /// Black: outlines, openings, shadowed parts.
    Black,
    /// Steel: tracks, wheels, barrels, bare metal.
    Steel,
    /// Pale: glass, lamps, highlights.
    Pale,
}

impl Material {
    fn named(name: &str) -> Option<Material> {
        Some(match name {
            "team-dark" => Material::TeamDark,
            "team" => Material::Team,
            "team-light" => Material::TeamLight,
            "black" => Material::Black,
            "steel" => Material::Steel,
            "pale" => Material::Pale,
            _ => return None,
        })
    }
}

/// Which face a sprite shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Facing {
    /// Its front, toward the viewer.
    Front,
    /// Its back.
    Rear,
    /// Its right side: its front points to the viewer's right.
    Right,
    /// Its left side.
    Left,
}

impl Facing {
    /// The face a contact row's `aspect` word names, front when it names
    /// none.
    #[must_use]
    pub fn from_aspect(word: &str) -> Facing {
        match word {
            "rear" => Facing::Rear,
            "right" => Facing::Right,
            "left" => Facing::Left,
            _ => Facing::Front,
        }
    }
}

/// One sprite frame: a material a pixel or nothing, row by row from the top.
pub type SpriteFrame = Vec<Option<Material>>;

/// One facing's frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacingFrames {
    /// Standing still.
    pub still: SpriteFrame,
    /// The moving cycle, two to four frames.
    pub moving: Vec<SpriteFrame>,
    /// Where each weapon's shot leaves, by weapon component id.
    pub muzzles: BTreeMap<String, (usize, usize)>,
}

/// A sprite sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sheet {
    /// Frame width, pixels.
    pub width: usize,
    /// Frame height, pixels.
    pub height: usize,
    /// How big the unit stands in the world, `(width, height)` centimetres.
    pub size_cm: (u32, u32),
    /// The frame pixel that stands on the ground, `(x, y)`.
    pub anchor: (usize, usize),
    /// The facings: front, rear, right, and left where the sheet gives it.
    pub facings: BTreeMap<Facing, FacingFrames>,
    /// The wreck, one or more frames.
    pub wreck: Vec<SpriteFrame>,
}

impl Sheet {
    /// The frame to draw for `facing`, moving or still at animation `step`,
    /// and whether it is drawn mirrored (a left face from the right one).
    #[must_use]
    pub fn frame(&self, facing: Facing, moving: bool, step: usize) -> Option<(&SpriteFrame, bool)> {
        let (frames, mirrored) = match self.facings.get(&facing) {
            Some(f) => (Some(f), false),
            None if facing == Facing::Left => (self.facings.get(&Facing::Right), true),
            None => (self.facings.get(&Facing::Front), false),
        };
        let frames = frames.or_else(|| self.facings.values().next())?;
        let cycle = frames.moving.len();
        let frame = if moving && cycle > 0 {
            frames.moving.get(step % cycle)?
        } else {
            &frames.still
        };
        Some((frame, mirrored))
    }

    /// The wreck frame for `facing`.
    #[must_use]
    pub fn wreck_frame(&self, facing: Facing) -> Option<&SpriteFrame> {
        let i = match facing {
            Facing::Front => 0,
            Facing::Rear => 1,
            Facing::Right => 2,
            Facing::Left => 3,
        };
        self.wreck.get(i % self.wreck.len().max(1))
    }
}

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
    /// The sprite sheet, where it has one.
    pub sprites: Option<Sheet>,
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
        let sprites = obj.get("sprites").map(parse_sheet).transpose()?;
        Ok(Look {
            units,
            label,
            short,
            weapons,
            doll,
            sprites,
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

fn pair(v: Option<&Value>, what: &str) -> Result<(u64, u64), LookError> {
    let a = v.and_then(Value::as_array);
    match a.map(Vec::as_slice) {
        Some([x, y]) => match (x.as_u64(), y.as_u64()) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => err(format!("sprites: `{what}` must be two whole numbers")),
        },
        _ => err(format!("sprites: `{what}` must be two whole numbers")),
    }
}

fn parse_sheet(v: &Value) -> Result<Sheet, LookError> {
    let Some(obj) = v.as_object() else {
        return err("sprites: must be an object");
    };
    only(
        obj,
        &["size", "size_cm", "anchor", "palette", "facings", "wreck"],
        "sprites",
    )?;
    let (w, h) = pair(obj.get("size"), "size")?;
    let (w, h) = (
        usize::try_from(w).unwrap_or(0),
        usize::try_from(h).unwrap_or(0),
    );
    if w == 0 || h == 0 || w > SPRITE_MAX || h > SPRITE_MAX {
        return err(format!(
            "sprites: frames {w}x{h}; at most {SPRITE_MAX}x{SPRITE_MAX}"
        ));
    }
    let (cw, ch) = pair(obj.get("size_cm"), "size_cm")?;
    if cw == 0 || ch == 0 {
        return err("sprites: `size_cm` must not be zero");
    }
    let (ax, ay) = pair(obj.get("anchor"), "anchor")?;
    let anchor = (
        usize::try_from(ax).unwrap_or(w),
        usize::try_from(ay).unwrap_or(h),
    );
    if anchor.0 >= w || anchor.1 >= h {
        return err("sprites: `anchor` must be inside the frame");
    }
    let Some(palette) = obj.get("palette").and_then(Value::as_object) else {
        return err("sprites: `palette` must be an object");
    };
    let mut keys: Vec<(char, Material)> = Vec::new();
    for (key, name) in palette {
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return err(format!(
                "sprites: palette key `{key}` must be one character"
            ));
        };
        if c == '.' {
            return err("sprites: `.` is nothing, not a palette key");
        }
        let Some(m) = name.as_str().and_then(Material::named) else {
            return err(format!(
                "sprites: palette `{key}` must name team-dark, team, team-light, black, steel or pale"
            ));
        };
        keys.push((c, m));
    }
    let frame = |v: &Value, what: &str| -> Result<SpriteFrame, LookError> {
        let rows: Vec<&str> = v
            .as_array()
            .and_then(|r| r.iter().map(Value::as_str).collect())
            .ok_or_else(|| LookError(format!("sprites: {what} must be a list of rows")))?;
        if rows.len() != h {
            return err(format!("sprites: {what} has {} rows, not {h}", rows.len()));
        }
        let mut out = Vec::with_capacity(w * h);
        for (y, row) in rows.iter().enumerate() {
            if row.chars().count() != w {
                return err(format!("sprites: {what} row {y} is not {w} wide"));
            }
            for c in row.chars() {
                if c == '.' {
                    out.push(None);
                } else {
                    match keys.iter().find(|(k, _)| *k == c) {
                        Some((_, m)) => out.push(Some(*m)),
                        None => {
                            return err(format!(
                                "sprites: {what} row {y} uses `{c}`, which the palette lacks"
                            ));
                        }
                    }
                }
            }
        }
        Ok(out)
    };
    let Some(facings_obj) = obj.get("facings").and_then(Value::as_object) else {
        return err("sprites: `facings` must be an object");
    };
    let mut facings = BTreeMap::new();
    for (name, f) in facings_obj {
        let facing = match name.as_str() {
            "front" => Facing::Front,
            "rear" => Facing::Rear,
            "right" => Facing::Right,
            "left" => Facing::Left,
            other => {
                return err(format!(
                    "sprites: facing `{other}` is not front, rear, right or left"
                ));
            }
        };
        let Some(fo) = f.as_object() else {
            return err(format!("sprites: facing `{name}` must be an object"));
        };
        only(fo, &["still", "moving", "muzzles"], "sprites facing")?;
        let still = frame(
            fo.get("still").unwrap_or(&Value::Null),
            &format!("{name} still"),
        )?;
        let moving: Vec<SpriteFrame> = match fo.get("moving").and_then(Value::as_array) {
            Some(list) if (2..=4).contains(&list.len()) => list
                .iter()
                .enumerate()
                .map(|(i, v)| frame(v, &format!("{name} moving {i}")))
                .collect::<Result<_, _>>()?,
            _ => {
                return err(format!(
                    "sprites: facing `{name}` needs two to four moving frames"
                ));
            }
        };
        let mut muzzles = BTreeMap::new();
        if let Some(m) = fo.get("muzzles") {
            let Some(m) = m.as_object() else {
                return err("sprites: `muzzles` must be an object");
            };
            for (weapon, at) in m {
                let (x, y) = pair(Some(at), "muzzle")?;
                let at = (
                    usize::try_from(x).unwrap_or(w),
                    usize::try_from(y).unwrap_or(h),
                );
                if at.0 >= w || at.1 >= h {
                    return err(format!(
                        "sprites: {name} muzzle `{weapon}` is outside the frame"
                    ));
                }
                muzzles.insert(weapon.clone(), at);
            }
        }
        facings.insert(
            facing,
            FacingFrames {
                still,
                moving,
                muzzles,
            },
        );
    }
    for need in [Facing::Front, Facing::Rear, Facing::Right] {
        if !facings.contains_key(&need) {
            return err(format!("sprites: the {need:?} facing is required").to_lowercase());
        }
    }
    let wreck: Vec<SpriteFrame> = match obj.get("wreck").and_then(Value::as_array) {
        Some(list) if (1..=4).contains(&list.len()) => list
            .iter()
            .enumerate()
            .map(|(i, v)| frame(v, &format!("wreck {i}")))
            .collect::<Result<_, _>>()?,
        _ => return err("sprites: `wreck` needs one to four frames"),
    };
    Ok(Sheet {
        width: w,
        height: h,
        size_cm: (
            u32::try_from(cw).unwrap_or(u32::MAX),
            u32::try_from(ch).unwrap_or(u32::MAX),
        ),
        anchor,
        facings,
        wreck,
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

/// The look an unclassified contact is drawn with ([`UNKNOWN`]).
#[must_use]
pub fn unknown() -> Option<&'static Look> {
    for_unit(UNKNOWN)
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

    /// Doc 22 cut U.3: a sheet's check refuses what §3.2 says it must, and
    /// its frames are picked by facing, motion and step — the left mirrored
    /// from the right, the moving cycle wrapping, the wreck by facing.
    #[test]
    fn a_sheet_is_checked_and_picks_its_frames() {
        let sheet = for_unit("tacsim.unit.biped_mk1")
            .and_then(|l| l.sprites.as_ref())
            .expect("the biped's sheet");
        assert_eq!(
            (sheet.width, sheet.height, sheet.anchor),
            (24, 50, (12, 49))
        );
        let (right, mirrored) = sheet.frame(Facing::Right, false, 0).expect("right");
        let (left, left_mirrored) = sheet.frame(Facing::Left, false, 0).expect("left");
        assert!(!mirrored && left_mirrored && std::ptr::eq(right, left));
        let cycle = &sheet.facings[&Facing::Front].moving;
        let (third, _) = sheet
            .frame(Facing::Front, true, 2 + cycle.len())
            .expect("moving");
        assert!(std::ptr::eq(third, &cycle[2]), "the cycle wraps");
        let (still, _) = sheet.frame(Facing::Front, false, 3).expect("still");
        assert!(std::ptr::eq(still, &sheet.facings[&Facing::Front].still));
        assert!(sheet.wreck_frame(Facing::Rear).is_some());

        let text = shipped_sources()
            .iter()
            .find(|(n, _)| *n == "biped.json")
            .map(|(_, t)| *t)
            .expect("the biped's look");
        let edited = |edit: &dyn Fn(&mut Value)| {
            let mut v: Value = serde_json::from_str(text).expect("JSON");
            edit(&mut v);
            Look::parse(&v.to_string())
        };
        let refused = |edit: &dyn Fn(&mut Value), why: &str| {
            let e = edited(edit).expect_err(why);
            assert!(e.0.contains(why), "{e} should say {why}");
        };
        refused(
            &|v| {
                v["sprites"]["facings"]
                    .as_object_mut()
                    .map(|f| f.remove("rear"));
            },
            "rear facing",
        );
        refused(
            &|v| v["sprites"]["palette"]["k"] = "purple".into(),
            "must name",
        );
        refused(
            &|v| v["sprites"]["facings"]["front"]["still"][0] = "...".into(),
            "not 24 wide",
        );
        refused(
            &|v| {
                let one = v["sprites"]["facings"]["front"]["moving"][0].clone();
                v["sprites"]["facings"]["front"]["moving"] = Value::Array(vec![one]);
            },
            "two to four",
        );
        refused(
            &|v| v["sprites"]["anchor"] = serde_json::json!([24, 49]),
            "inside the frame",
        );
        refused(&|v| v["sprites"]["wreck"] = serde_json::json!([]), "wreck");
        assert!(edited(&|_| {}).is_ok());
    }

    /// Doc 22 cut U.5: every shipped sheet holds together as art, past what
    /// the check refuses — every frame of a sheet is one size with the anchor
    /// and every muzzle inside it (the check's rules, held here against the
    /// parsed sheets), each facing's still frame stands on its anchor (a
    /// drawn pixel on the anchor's row or the one above it, so the unit
    /// meets its ground shadow), every muzzle sits on a drawn pixel (where
    /// the weapon is, not in the air beside it), the moving cycle visibly
    /// moves (no two frames of it the same) and the wreck is not the still.
    #[test]
    fn every_shipped_sheet_stands_moves_and_wrecks() {
        for (name, text) in SHIPPED {
            let look = Look::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let Some(sheet) = look.sprites.as_ref() else {
                continue;
            };
            let (w, h, (ax, ay)) = (sheet.width, sheet.height, sheet.anchor);
            assert!(ax < w && ay < h, "{name}: the anchor is inside");
            let drawn = |frame: &SpriteFrame, x: usize, y: usize| {
                frame.get(y * w + x).is_some_and(Option::is_some)
            };
            for (facing, frames) in &sheet.facings {
                for frame in std::iter::once(&frames.still).chain(&frames.moving) {
                    assert_eq!(frame.len(), w * h, "{name}: {facing:?} frames are {w}x{h}");
                }
                assert!(
                    drawn(&frames.still, ax, ay) || (ay > 0 && drawn(&frames.still, ax, ay - 1)),
                    "{name}: the {facing:?} still stands on its anchor"
                );
                for (weapon, &(mx, my)) in &frames.muzzles {
                    assert!(
                        mx < w && my < h,
                        "{name}: {facing:?} muzzle {weapon} is inside"
                    );
                    assert!(
                        drawn(&frames.still, mx, my),
                        "{name}: {facing:?} muzzle {weapon} is on the weapon"
                    );
                }
                for (i, a) in frames.moving.iter().enumerate() {
                    for (j, b) in frames.moving.iter().enumerate().skip(i + 1) {
                        assert!(a != b, "{name}: the {facing:?} cycle moves ({i} is {j})");
                    }
                }
            }
            for (i, wreck) in sheet.wreck.iter().enumerate() {
                assert_eq!(wreck.len(), w * h, "{name}: wreck {i} is {w}x{h}");
                assert!(
                    drawn(wreck, ax, ay) || (ay > 0 && drawn(wreck, ax, ay - 1)),
                    "{name}: wreck {i} lies on the anchor"
                );
                for (facing, frames) in &sheet.facings {
                    assert!(
                        *wreck != frames.still,
                        "{name}: wreck {i} is plainly a wreck, not the {facing:?} still"
                    );
                }
            }
        }
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
