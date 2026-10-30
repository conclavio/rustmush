//! **A unit's look** (doc 22, the unit kit): how a unit type is drawn, kept
//! apart from its rules (D-22-1) — the rules pack never carries art and art
//! never moves a rules digest. A look names the unit types it draws and
//! carries:
//!
//! - `label` and `short`: what a screen prints for the type;
//! - `size_cm`: how big the unit stands in the world, `(width, height)`;
//! - `model` (doc 31 D-31-1): the baked model it is drawn from, an id in
//!   [`crate::model`]'s catalogue — absent for a hand-drawn look;
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
//!   frames, the frame's size in pixels, the pixel that stands on the
//!   ground, a muzzle point a weapon a facing. A look on a model carries
//!   none: it draws the model's sheet ([`Look::sheet`]).
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
/// The largest sprite frame, pixels a side (doc 22 D-22-9: 96x96 frames).
pub const SPRITE_MAX: usize = 96;
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
    ("heavy.json", include_str!("../looks/heavy.json")),
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

/// Which of the eight faces a sprite shows (doc 22 D-22-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Facing {
    /// Its front, toward the viewer.
    Front,
    /// Between its front and its right side.
    FrontRight,
    /// Its right side: its front points to the viewer's right.
    Right,
    /// Between its back and its right side.
    RearRight,
    /// Its back.
    Rear,
    /// Between its back and its left side.
    RearLeft,
    /// Its left side.
    Left,
    /// Between its front and its left side.
    FrontLeft,
}

impl Facing {
    /// All eight facings in clockwise order from `front`.
    pub const ALL: [Facing; 8] = [
        Facing::Front,
        Facing::FrontRight,
        Facing::Right,
        Facing::RearRight,
        Facing::Rear,
        Facing::RearLeft,
        Facing::Left,
        Facing::FrontLeft,
    ];

    /// The face a contact row's `facing` (or four-way `aspect`) word names,
    /// `front` when it names none.
    #[must_use]
    pub fn from_aspect(word: &str) -> Facing {
        Self::named(word).unwrap_or(Facing::Front)
    }

    /// Parse an exact eight-way facing name.
    #[must_use]
    pub fn named(word: &str) -> Option<Facing> {
        Some(match word {
            "front" => Facing::Front,
            "front_right" => Facing::FrontRight,
            "right" => Facing::Right,
            "rear_right" => Facing::RearRight,
            "rear" => Facing::Rear,
            "rear_left" => Facing::RearLeft,
            "left" => Facing::Left,
            "front_left" => Facing::FrontLeft,
            _ => return None,
        })
    }

    /// The word this facing is written as in a sheet or cockpit frame.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Facing::Front => "front",
            Facing::FrontRight => "front_right",
            Facing::Right => "right",
            Facing::RearRight => "rear_right",
            Facing::Rear => "rear",
            Facing::RearLeft => "rear_left",
            Facing::Left => "left",
            Facing::FrontLeft => "front_left",
        }
    }

    /// The nearest cardinal facing, used when a four-facing hand-drawn sheet
    /// is drawn at a diagonal contact facing.
    #[must_use]
    pub const fn cardinal(self) -> Facing {
        match self {
            Facing::Front => Facing::Front,
            Facing::FrontRight | Facing::Right | Facing::RearRight => Facing::Right,
            Facing::Rear => Facing::Rear,
            Facing::RearLeft | Facing::Left | Facing::FrontLeft => Facing::Left,
        }
    }
}

/// One sprite frame: a material a pixel or nothing, row by row from the top.
pub type SpriteFrame = Vec<Option<Material>>;

/// One facing's frames (doc 22 §3.2, D-22-9: up to six poses — `still`,
/// two to four `moving` frames, and an optional per-facing `wreck`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacingFrames {
    /// Standing still.
    pub still: SpriteFrame,
    /// The moving cycle, two to four frames.
    pub moving: Vec<SpriteFrame>,
    /// The wreck frame for this facing, when the sheet gives one per facing
    /// (doc 22 D-22-9).
    pub wreck: Option<SpriteFrame>,
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
    /// The frame pixel that stands on the ground, `(x, y)`.
    pub anchor: (usize, usize),
    /// The facings: all eight on a D-22-9 baked sheet, or front/rear/right
    /// (and optional left) on a hand-drawn sheet.
    pub facings: BTreeMap<Facing, FacingFrames>,
    /// Top-level wreck frames (hand-drawn sheets); empty when each facing
    /// carries its own `wreck` in [`FacingFrames::wreck`].
    pub wreck: Vec<SpriteFrame>,
}

impl Sheet {
    /// Resolve `facing` against the facings this sheet carries, falling back
    /// from a missing diagonal to its cardinal side and mirroring `right`
    /// only when `left` itself is absent.
    fn resolve_facing(&self, facing: Facing) -> Option<(&FacingFrames, bool)> {
        if let Some(f) = self.facings.get(&facing) {
            return Some((f, false));
        }
        let cardinal = facing.cardinal();
        if let Some(f) = self.facings.get(&cardinal) {
            return Some((f, false));
        }
        if cardinal == Facing::Left
            && let Some(f) = self.facings.get(&Facing::Right)
        {
            return Some((f, true));
        }
        self.facings
            .get(&Facing::Front)
            .or_else(|| self.facings.values().next())
            .map(|f| (f, false))
    }

    /// The frame to draw for `facing`, moving or still at animation `step`,
    /// and whether it is drawn mirrored (a left face from the right one on a
    /// sheet that omits `left`).
    #[must_use]
    pub fn frame(&self, facing: Facing, moving: bool, step: usize) -> Option<(&SpriteFrame, bool)> {
        let (frames, mirrored) = self.resolve_facing(facing)?;
        let cycle = frames.moving.len();
        let frame = if moving && cycle > 0 {
            frames.moving.get(step % cycle)?
        } else {
            &frames.still
        };
        Some((frame, mirrored))
    }

    /// The wreck frame for `facing`: the facing's own `wreck` pose when the
    /// sheet gives one (doc 22 D-22-9), else indexed from top-level `wreck`.
    #[must_use]
    pub fn wreck_frame(&self, facing: Facing) -> Option<&SpriteFrame> {
        if let Some((frames, _)) = self.resolve_facing(facing)
            && let Some(wreck) = frames.wreck.as_ref()
        {
            return Some(wreck);
        }
        if self.wreck.is_empty() {
            return None;
        }
        let i = match facing.cardinal() {
            Facing::Front => 0,
            Facing::Rear => 1,
            Facing::Right => 2,
            Facing::Left => 3,
            _ => 0,
        };
        self.wreck.get(i % self.wreck.len())
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
    /// The look's id: its file's stem among the shipped looks (`biped`,
    /// `tracked_tank`, …); empty for a look parsed from text alone. Its bake
    /// is keyed by its [`Look::model`] (doc 31 D-31-1), not by this id.
    pub id: String,
    /// The unit types (definition ids) it draws.
    pub units: Vec<String>,
    /// The model it is drawn from (doc 31 D-31-1), an id in
    /// [`crate::model`]'s catalogue; `None` for a hand-drawn look, which
    /// draws its own sheet.
    pub model: Option<String>,
    /// How big the unit stands in the world, `(width, height)` centimetres
    /// (on the look since doc 31: a look with no sheet of its own has one).
    pub size_cm: (u32, u32),
    /// The type's name.
    pub label: String,
    /// Its short name.
    pub short: String,
    /// A short name a weapon component id.
    pub weapons: BTreeMap<String, String>,
    /// The doll, where it has one.
    pub doll: Option<Doll>,
    /// Its own sprite sheet, where it has one (a hand-drawn unit's; a look
    /// on a model draws the model's, [`Look::sheet`]).
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
            &[
                "units", "label", "short", "model", "size_cm", "weapons", "doll", "sprites",
            ],
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
        let model = match obj.get("model") {
            None => None,
            Some(_) => Some(string(obj, "model", "look")?),
        };
        let (cw, ch) = pair(obj.get("size_cm"), "size_cm")
            .map_err(|_| LookError("look: `size_cm` must be two whole numbers".into()))?;
        if cw == 0 || ch == 0 {
            return err("look: `size_cm` must not be zero");
        }
        let size_cm = (
            u32::try_from(cw).unwrap_or(u32::MAX),
            u32::try_from(ch).unwrap_or(u32::MAX),
        );
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
            id: String::new(),
            units,
            model,
            size_cm,
            label,
            short,
            weapons,
            doll,
            sprites,
        })
    }

    /// **The sheet the look draws** when no bake is drawn: its own, else its
    /// model's fallback sheet ([`crate::model::sheet`], doc 31 D-31-1).
    #[must_use]
    pub fn sheet(&self) -> Option<&Sheet> {
        self.sprites
            .as_ref()
            .or_else(|| self.model.as_deref().and_then(crate::model::sheet))
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

/// **Read and check one sheet** given as its own JSON text — a model's
/// fallback sheet under `models/` (doc 31 D-31-1) — by the rules a look's
/// `sprites` is read by.
///
/// # Errors
///
/// [`LookError`] naming the first thing that does not hold.
pub fn parse_sheet_text(text: &str) -> Result<Sheet, LookError> {
    let v: Value = serde_json::from_str(text).map_err(|e| LookError(format!("not JSON: {e}")))?;
    parse_sheet(&v)
}

fn parse_sheet(v: &Value) -> Result<Sheet, LookError> {
    let Some(obj) = v.as_object() else {
        return err("sprites: must be an object");
    };
    only(
        obj,
        &["size", "anchor", "palette", "facings", "wreck"],
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
        let Some(facing) = Facing::named(name.as_str()) else {
            return err(format!(
                "sprites: facing `{name}` is not front, front_right, right, rear_right, rear, rear_left, left or front_left"
            ));
        };
        let Some(fo) = f.as_object() else {
            return err(format!("sprites: facing `{name}` must be an object"));
        };
        only(
            fo,
            &["still", "moving", "wreck", "muzzles"],
            "sprites facing",
        )?;
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
        let facing_wreck = match fo.get("wreck") {
            Some(v) => Some(frame(v, &format!("{name} wreck"))?),
            None => None,
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
                wreck: facing_wreck,
                muzzles,
            },
        );
    }
    for need in [Facing::Front, Facing::Rear, Facing::Right] {
        if !facings.contains_key(&need) {
            return err(format!("sprites: the {need:?} facing is required").to_lowercase());
        }
    }
    // D-22-9: if any diagonal is given, all eight facings must be given (never
    // a mirrored left on an asymmetric baked sheet).
    let has_diagonal = facings.keys().any(|f| f.cardinal() != *f);
    if has_diagonal {
        for need in Facing::ALL {
            if !facings.contains_key(&need) {
                return err(format!(
                    "sprites: eight-facing sheet is missing `{}`",
                    need.as_str()
                ));
            }
        }
    }
    let all_have_facing_wreck = !facings.is_empty() && facings.values().all(|f| f.wreck.is_some());
    let wreck: Vec<SpriteFrame> = match obj.get("wreck") {
        Some(v) => match v.as_array() {
            Some(list) if (1..=8).contains(&list.len()) => list
                .iter()
                .enumerate()
                .map(|(i, v)| frame(v, &format!("wreck {i}")))
                .collect::<Result<_, _>>()?,
            _ => return err("sprites: `wreck` needs one to eight frames"),
        },
        None if all_have_facing_wreck => Vec::new(),
        None => return err("sprites: `wreck` needs one to eight frames (or a `wreck` per facing)"),
    };
    Ok(Sheet {
        width: w,
        height: h,
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
            .filter_map(|(name, text)| {
                let mut look = Look::parse(text).ok()?;
                look.id = name.strip_suffix(".json").unwrap_or(name).to_string();
                Some(look)
            })
            .collect()
    })
}

/// The looks installed at run time (doc 31 cut UD.3b, D-31-16): published
/// designs' looks, learned from the `unit.looks` feed. Append-only, each
/// leaked once, so every reader keeps a `'static` look and an index never
/// moves.
static RUNTIME: std::sync::RwLock<Vec<&'static Look>> = std::sync::RwLock::new(Vec::new());

fn runtime() -> Vec<&'static Look> {
    RUNTIME
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// **Install a published design's look** for unit type `unit` (doc 31 cut
/// UD.3b): parsed and checked as a shipped look is, its id the unit type; a
/// unit that already has a look (shipped or installed) keeps it. Whether it
/// was installed.
///
/// # Errors
///
/// [`LookError`] when the look does not parse, or does not name `unit`.
pub fn install(unit: &str, text: &str) -> Result<bool, LookError> {
    let mut look = Look::parse(text)?;
    if !look.units.iter().any(|u| u == unit) {
        return err(format!("look: it draws {:?}, not {unit}", look.units));
    }
    if for_unit(unit).is_some() {
        return Ok(false);
    }
    look.id = unit.to_string();
    RUNTIME
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Box::leak(Box::new(look)));
    Ok(true)
}

/// The look at `index` among every look this process draws: the shipped ones
/// in [`shipped`]'s order, then those [`install`]ed, in install order.
#[must_use]
pub fn by_index(index: usize) -> Option<&'static Look> {
    let shipped = shipped();
    match shipped.get(index) {
        Some(look) => Some(look),
        None => runtime().get(index - shipped.len()).copied(),
    }
}

/// The index ([`by_index`]) of the look that draws unit type `unit`.
#[must_use]
pub fn index_for_unit(unit: &str) -> Option<usize> {
    let shipped = shipped();
    shipped
        .iter()
        .position(|l| l.units.iter().any(|u| u == unit))
        .or_else(|| {
            runtime()
                .iter()
                .position(|l| l.units.iter().any(|u| u == unit))
                .map(|i| shipped.len() + i)
        })
}

/// The look that draws unit type `unit`, if any: a shipped one, else one
/// [`install`]ed.
#[must_use]
pub fn for_unit(unit: &str) -> Option<&'static Look> {
    shipped()
        .iter()
        .find(|l| l.units.iter().any(|u| u == unit))
        .or_else(|| {
            runtime()
                .into_iter()
                .find(|l| l.units.iter().any(|u| u == unit))
        })
}

/// **The unit types a client still needs a look for** (doc 31 cut UD.3b):
/// each named type with no shipped or installed look, once each, in the
/// order first named. A type with no look anywhere (a hauler) is named too;
/// the caller asks once and remembers.
#[must_use]
pub fn wanted<'a>(types: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for unit in types {
        if unit != UNKNOWN && for_unit(unit).is_none() && !out.contains(&unit) {
            out.push(unit);
        }
    }
    out
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
        let base = |doll: &str| {
            format!(
                r#"{{"units":["u"],"label":"U","short":"U","size_cm":[100,100],"doll":{doll}}}"#
            )
        };
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
            r#"{"units":["u"],"label":"U","short":"U","size_cm":[1,1],"colour":"red"}"#.to_string(),
            "unknown key",
        );
        refused(
            r#"{"units":[],"label":"U","short":"U","size_cm":[1,1]}"#.to_string(),
            "at least one",
        );
        refused(
            r#"{"units":["u"],"label":"U","short":"U"}"#.to_string(),
            "size_cm",
        );
        refused(
            r#"{"units":["u"],"label":"U","short":"U","size_cm":[0,1]}"#.to_string(),
            "not be zero",
        );
        refused(
            r#"{"units":["u"],"label":"U","short":"U","size_cm":[1,1],"model":""}"#.to_string(),
            "model",
        );
    }

    /// Doc 22 cut U.3: a sheet's check refuses what §3.2 says it must, and
    /// its frames are picked by facing, motion and step — the left mirrored
    /// from the right, the moving cycle wrapping, the wreck by facing.
    #[test]
    fn a_sheet_is_checked_and_picks_its_frames() {
        // The biped's sheet is its model's (doc 31 D-31-1).
        let sheet = for_unit("tacsim.unit.biped_mk1")
            .and_then(Look::sheet)
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

        let text = crate::model::sheet_sources()
            .iter()
            .find(|(n, _)| *n == "mc.lt.fox_biped.sheet.json")
            .map(|(_, t)| *t)
            .expect("the biped's sheet");
        let edited = |edit: &dyn Fn(&mut Value)| {
            let mut v = serde_json::json!({ "sprites": serde_json::from_str::<Value>(text).expect("JSON") });
            edit(&mut v);
            parse_sheet_text(&v["sprites"].to_string())
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

        // D-22-9: an eight-facing, six-pose, 96x96 sheet with per-facing wreck
        // parses, picks every diagonal without mirroring, and refuses a sheet
        // past 96 or missing one of the eight facings.
        let row96 = format!("a{}", ".".repeat(95));
        let frame96: Vec<String> = (0..96).map(|_| row96.clone()).collect();
        let mut facings8 = serde_json::Map::new();
        for f in Facing::ALL {
            facings8.insert(
                f.as_str().to_string(),
                serde_json::json!({
                    "still": frame96,
                    "moving": [frame96, frame96, frame96, frame96],
                    "wreck": frame96,
                    "muzzles": { "cannon": [0, 0] }
                }),
            );
        }
        let d22_9 = serde_json::json!({
            "units": ["tacsim.unit.biped_mk1"],
            "label": "Biped 96",
            "short": "BP",
            "size_cm": [520, 1100],
            "sprites": {
                "size": [96, 96],
                "anchor": [0, 95],
                "palette": { "a": "team" },
                "facings": facings8,
            }
        });
        let parsed = Look::parse(&d22_9.to_string()).expect("D-22-9 96x96 8-facing sheet");
        let s8 = parsed.sprites.as_ref().expect("sprites");
        assert_eq!((s8.width, s8.height), (96, 96));
        for f in Facing::ALL {
            let (fr, mirrored) = s8.frame(f, false, 0).expect("facing");
            assert!(!mirrored, "{f:?} has its own art");
            assert!(std::ptr::eq(fr, &s8.facings[&f].still));
            let wr = s8.wreck_frame(f).expect("wreck");
            assert!(std::ptr::eq(
                wr,
                s8.facings[&f].wreck.as_ref().expect("wreck")
            ));
        }
        let mut missing_diag = d22_9.clone();
        missing_diag["sprites"]["facings"]
            .as_object_mut()
            .and_then(|m| m.remove("front_left"));
        let err = Look::parse(&missing_diag.to_string()).expect_err("missing front_left");
        assert!(err.0.contains("front_left"), "{err}");
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
        // The looks' own sheets and the models' (doc 31 D-31-1): every sheet
        // the tree ships.
        let mut sheets: Vec<(String, Sheet)> = SHIPPED
            .iter()
            .filter_map(|(name, text)| {
                let look = Look::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
                Some((name.to_string(), look.sprites?))
            })
            .collect();
        for (name, text) in crate::model::sheet_sources() {
            let sheet = parse_sheet_text(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            sheets.push((name.to_string(), sheet));
        }
        assert!(sheets.len() >= 6, "every model has its sheet");
        for (name, sheet) in &sheets {
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
