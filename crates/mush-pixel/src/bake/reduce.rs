//! **The reducer** (doc 22 D-22-10, rules 1 and 2): one model's bake — the
//! 384 px colour-ID and normal renders `Bake.cs` writes for both views, eight
//! facings and six poses, and its `meta.json` — reduced to the model's pack
//! entry (doc 31 D-31-1). It is the only resampler: every size of both chains is reduced from
//! the large render, never from a smaller size, as masses (a cell is filled
//! when 30 % of its render pixels are, so thin parts survive; its material is
//! the most common among them).
//!
//! The **material** of a render pixel is its part (the colour-ID pass: three
//! bits of red, green and blue name the part, [`Part`]): the main paint the
//! team's colour, secondary panels steel, dark metal black, accents
//! team-light, glass pale; the **mask pass** (`_m`; red the kit's metallic
//! map, green a rubber flag from the material's name — tracks, tyres — blue
//! the kit's emission map) makes a flagged pixel rubber whatever its part and
//! a lit pixel a lamp. (The metallic map is carried but unused: this kit
//! paints nine tenths of every part dielectric, so it tells nothing apart.)
//!
//! The **shade** of a render pixel (format 5, the shade plane, U.6 follow-up
//! of 2026-11-06) is the bake's **lit pass** (`_l`: the kit's surface under a
//! key light and a fill through its normal map, times its albedo's
//! luminance for the painted detail), darkened by occlusion from the **depth
//! pass** (`_d`), levelled a material over the whole model (so a red paint's
//! ramp runs its full span whatever its albedo's luminance) and quantised to
//! fifteen steps; a pixel on a crease or a step between parts is the **ink
//! line**, shade `0`. A bake without the lit pass shades by the normal pass
//! under the same key light; without the depth pass, no occlusion and
//! creases only. A cell of the reduced frame takes the most common material
//! of its render pixels and the mean shade of that material's, or the ink
//! when four tenths of them are ink.
//!
//! `examples/unit_reduce.rs` runs it over a bake directory and writes the
//! pack; `scripts/unit-bake/bake.sh` runs the bake and then it.

use serde_json::Value;

use super::{
    BakedFrame, Chain, FRAMES, ModelBake, POSES, Provenance, RENDER_PX, SizedFrames, View,
};
use crate::look::Facing;

/// A part of the model, as its colour-ID pass names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The main paint: the team's three tones by light.
    Paint,
    /// Secondary panels: the three steel tones by light, at the paint's
    /// cuts (0.72 and 0.35), so bare metal models its form as the paint
    /// does. A face square to the viewer is lit 0.45 under the fixed light,
    /// so a cut at 0.45 — the first rule — split every such face on noise.
    Second,
    /// Dark metal: black.
    Dark,
    /// Accent stripes: team-light.
    Accent,
    /// Lamps, glass, hazard paint: pale.
    Bright,
}

impl Part {
    fn named(word: &str) -> Option<Part> {
        Some(match word {
            "paint" => Part::Paint,
            "second" => Part::Second,
            "dark" => Part::Dark,
            "accent" => Part::Accent,
            "bright" => Part::Bright,
            _ => return None,
        })
    }
}

/// The colour-ID bits (`r·4 + g·2 + b`, each channel over half) to a part,
/// the Mech Constructor kit's convention: black dark metal, blue secondary,
/// green paint, red accents, the rest bright. A bake's `meta.json` may give
/// its own (`id_classes`).
pub const DEFAULT_ID_CLASSES: [Part; 8] = [
    Part::Dark,
    Part::Second,
    Part::Paint,
    Part::Bright,
    Part::Accent,
    Part::Bright,
    Part::Bright,
    Part::Bright,
];

/// The fraction of a cell's render pixels that must be drawn for the cell to
/// be, in tenths: three (masses, and thin parts survive).
const COVERAGE_TENTHS: usize = 3;

/// Classify one render: each pixel's material code (`0` nothing, else a
/// family base — `2` the team's colour, `5` steel, `9` rubber — or a fixed
/// material) from its colour-ID pixels (RGBA), the mask pixels when the
/// bake has them (green rubber, blue emission), with the part table
/// `classes`. The normal pass is read only for the pixel's presence.
#[must_use]
pub fn classify(
    id_rgba: &[u8],
    normal_rgba: &[u8],
    mask_rgba: Option<&[u8]>,
    classes: &[Part; 8],
) -> Vec<u8> {
    let mask = mask_rgba.map(|m| m.as_chunks::<4>().0);
    id_rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(normal_rgba.as_chunks::<4>().0)
        .enumerate()
        .map(|(i, (&[r, g, b, a], _))| {
            if a <= 127 {
                return 0;
            }
            let bits = usize::from(r > 127) * 4 + usize::from(g > 127) * 2 + usize::from(b > 127);
            let (rubber, emission) = mask
                .and_then(|m| m.get(i))
                .map_or((false, false), |&[_, rb, em, _]| (rb > 127, em > 64));
            match classes.get(bits).copied().unwrap_or(Part::Dark) {
                _ if emission => 11,
                _ if rubber => 9,
                Part::Paint => 2,
                Part::Second => 5,
                Part::Dark => 4,
                Part::Accent => 3,
                Part::Bright => 6,
            }
        })
        .collect()
}

/// The key light, viewer-relative and normalised: upper left, toward the
/// viewer (the camera looks along +Z). The lit pass uses the same.
fn key_light() -> [f32; 3] {
    let (lx, ly, lz) = (-0.4f32, 0.8f32, -0.45f32);
    let len = (lx * lx + ly * ly + lz * lz).sqrt();
    [lx / len, ly / len, lz / len]
}

/// A render pixel's lit value from the world-normal pass alone: what shades
/// a bake without the lit pass (`0.18` ambient plus the key light).
#[must_use]
pub fn shade_from_normal(normal_rgba: &[u8]) -> Vec<f32> {
    let light = key_light();
    let unit = |v: u8| f32::from(v) / 255.0 * 2.0 - 1.0;
    normal_rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[nx, ny, nz, _]| {
            let lit =
                (unit(nx) * light[0] + unit(ny) * light[1] + unit(nz) * light[2]).clamp(0.0, 1.0);
            0.18 + 0.82 * lit
        })
        .collect()
}

/// The depth pass decoded: metres from the camera (its red the high byte,
/// green the low, over 150 to 250 m), infinity where nothing was drawn.
#[must_use]
pub fn depth_metres(depth_rgba: &[u8]) -> Vec<f32> {
    depth_rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[hi, lo, _, a]| {
            if a <= 127 {
                f32::INFINITY
            } else {
                150.0 + 100.0 * ((f32::from(hi) + f32::from(lo) / 255.0) / 255.0)
            }
        })
        .collect()
}

/// Occlusion from the depth plane: `1` in the open, down to `0.5` in a
/// crease — the share of eight neighbours, six pixels out, that stand nearer
/// the camera by over 0.3 m.
#[must_use]
pub fn occlusion(depth: &[f32], res: usize) -> Vec<f32> {
    const RADIUS: i64 = 6;
    const STEP_M: f32 = 0.3;
    let at = |x: i64, y: i64| -> Option<f32> {
        if x < 0 || y < 0 || x >= res as i64 || y >= res as i64 {
            return None;
        }
        depth
            .get(usize::try_from(y * res as i64 + x).ok()?)
            .copied()
            .filter(|d| d.is_finite())
    };
    (0..res * res)
        .map(|i| {
            let (x, y) = ((i % res) as i64, (i / res) as i64);
            let Some(d) = at(x, y) else {
                return 1.0;
            };
            let mut nearer = 0;
            for (dx, dy) in [
                (1, 0),
                (1, 1),
                (0, 1),
                (-1, 1),
                (-1, 0),
                (-1, -1),
                (0, -1),
                (1, -1),
            ] {
                if at(x + dx * RADIUS, y + dy * RADIUS).is_some_and(|n| n < d - STEP_M) {
                    nearer += 1;
                }
            }
            1.0 - 0.5 * (nearer as f32 / 8.0)
        })
        .collect()
}

/// The ink line: a drawn pixel on a crease (a neighbour's normal turned past
/// about 57°) or, with the depth plane, on a step between parts (a
/// neighbour over 0.5 m nearer or farther).
#[must_use]
pub fn ink(normal_rgba: &[u8], depth: Option<&[f32]>, res: usize) -> Vec<bool> {
    const CREASE: f32 = 0.55;
    const STEP_M: f32 = 0.5;
    let normals = normal_rgba.as_chunks::<4>().0;
    let unit = |v: u8| f32::from(v) / 255.0 * 2.0 - 1.0;
    let normal = |x: i64, y: i64| -> Option<[f32; 3]> {
        if x < 0 || y < 0 || x >= res as i64 || y >= res as i64 {
            return None;
        }
        let &[nx, ny, nz, a] = normals.get(usize::try_from(y * res as i64 + x).ok()?)?;
        (a > 127).then_some([unit(nx), unit(ny), unit(nz)])
    };
    (0..res * res)
        .map(|i| {
            let (x, y) = ((i % res) as i64, (i / res) as i64);
            let Some(n0) = normal(x, y) else {
                return false;
            };
            let d0 = depth
                .and_then(|d| d.get(i).copied())
                .unwrap_or(f32::INFINITY);
            [(1, 0), (0, 1), (-1, 0), (0, -1)].iter().any(|&(dx, dy)| {
                let Some(n1) = normal(x + dx, y + dy) else {
                    return false;
                };
                let dot = n0[0] * n1[0] + n0[1] * n1[1] + n0[2] * n1[2];
                if dot < CREASE {
                    return true;
                }
                let j = usize::try_from((y + dy) * res as i64 + x + dx).unwrap_or(usize::MAX);
                let d1 = depth
                    .and_then(|d| d.get(j).copied())
                    .unwrap_or(f32::INFINITY);
                d0.is_finite() && d1.is_finite() && (d0 - d1).abs() > STEP_M
            })
        })
        .collect()
}

/// Every render pixel's shade value (unlevelled) and whether it is ink: the
/// lit pass's light times its albedo luminance (`0.5` plus the luminance),
/// else the normal pass's light; times the depth pass's occlusion.
#[must_use]
pub fn shade_values(
    normal_rgba: &[u8],
    lit_rgba: Option<&[u8]>,
    depth: Option<&[f32]>,
    res: usize,
) -> (Vec<f32>, Vec<bool>) {
    let base: Vec<f32> = match lit_rgba {
        Some(l) => l
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[lit, lum, _, _]| f32::from(lit) / 255.0 * (0.5 + f32::from(lum) / 255.0))
            .collect(),
        None => shade_from_normal(normal_rgba),
    };
    let ao = depth.map(|d| occlusion(d, res));
    let values = base
        .iter()
        .enumerate()
        .map(|(i, &v)| v * ao.as_ref().and_then(|a| a.get(i)).copied().unwrap_or(1.0))
        .collect();
    (values, ink(normal_rgba, depth, res))
}

/// The span of shade values the levels histogram covers.
const SHADE_SPAN: f32 = 1.5;

/// A material's levels from its shade histogram: the 2nd and 98th
/// percentiles, at least 0.05 apart.
fn percentile_levels(h: &[u32; 256]) -> (f32, f32) {
    let total: u32 = h.iter().sum();
    if total == 0 {
        return (0.0, 1.0);
    }
    let at = |p: f32| -> f32 {
        let want = (total as f32 * p) as u32;
        let mut seen = 0u32;
        for (bin, &k) in h.iter().enumerate() {
            seen += k;
            if seen >= want {
                return bin as f32 / 255.0 * SHADE_SPAN;
            }
        }
        SHADE_SPAN
    };
    let lo = at(0.02);
    let hi = at(0.98).max(lo + 0.05);
    (lo, hi)
}

/// A shade value to its step, `1` darkest to `15` lightest, between levels.
#[must_use]
pub fn quantise(value: f32, levels: (f32, f32)) -> u8 {
    let s = ((value - levels.0) / (levels.1 - levels.0)).clamp(0.0, 1.0);
    1 + (s * 14.0).round() as u8
}

/// **Reduce** a `res` by `res` grid of pixels (material and shade) to `n` by
/// `n` as masses: a cell covers the render pixels between its edges (`c·res/n`
/// to `(c+1)·res/n`), is drawn when at least three tenths of them are, takes
/// the most common material among them (the lower code on a tie) and the
/// mean shade of that material's pixels — or the ink line when four tenths
/// of them are ink.
#[must_use]
pub fn reduce(hi: &[u8], res: usize, n: usize) -> Vec<u8> {
    const INK_TENTHS: usize = 4;
    let mut out = vec![0u8; n * n];
    if n == 0 || res < n {
        return out;
    }
    for (r, row) in out.chunks_exact_mut(n).enumerate() {
        let (y0, y1) = (r * res / n, (r + 1) * res / n);
        for (c, cell) in row.iter_mut().enumerate() {
            let (x0, x1) = (c * res / n, (c + 1) * res / n);
            // One slot a code format 5 can carry; a vote that stopped at six
            // once collapsed every later material into pale.
            let mut counts = [0usize; 16];
            let mut shade_sum = [0usize; 16];
            let mut inked = [0usize; 16];
            for y in y0..y1 {
                for x in x0..x1 {
                    let b = hi.get(y * res + x).copied().unwrap_or(0);
                    let code = usize::from(b & 0x0f);
                    counts[code] += 1;
                    if b >> 4 == 0 {
                        inked[code] += 1;
                    } else {
                        shade_sum[code] += usize::from(b >> 4);
                    }
                }
            }
            let total = (y1 - y0) * (x1 - x0);
            let filled: usize = counts.iter().skip(1).sum();
            if filled * 10 < total * COVERAGE_TENTHS || filled == 0 {
                continue;
            }
            let mut best = (0usize, 0usize);
            for (code, &k) in counts.iter().enumerate().skip(1) {
                if k > best.0 {
                    best = (k, code);
                }
            }
            let (k, code) = best;
            let shaded = k - inked[code];
            let shade = if inked[code] * 10 >= k * INK_TENTHS || shaded == 0 {
                0
            } else {
                u8::try_from((shade_sum[code] + shaded / 2) / shaded).unwrap_or(15)
            };
            *cell = u8::try_from(code).unwrap_or(0) | (shade << 4);
        }
    }
    out
}

/// One frame's marks as the bake measured them, render pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaFrame {
    /// The facing.
    pub facing: Facing,
    /// The pose index ([`POSES`]).
    pub pose: usize,
    /// The ground point under the unit's centre.
    pub anchor: (f32, f32),
    /// Each weapon's muzzle, by weapon component id.
    pub muzzles: Vec<(String, (f32, f32))>,
}

/// One view of the bake.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaView {
    /// The view.
    pub view: View,
    /// The world span of the render's side, centimetres.
    pub span_cm: u32,
    /// The 48 frames' marks.
    pub frames: Vec<MetaFrame>,
}

/// A bake's `meta.json` (format 3, written by `Bake.cs`: format 2 named a
/// look, format 3 the model, doc 31 D-31-1).
#[derive(Clone, Debug, PartialEq)]
pub struct BakeMeta {
    /// The model baked, by id.
    pub model: String,
    /// The render's side, pixels.
    pub res: usize,
    /// The baked model's `(width, height)`, centimetres.
    pub size_cm: (u32, u32),
    /// What it was made from (the reduce commit is the reducer's to fill).
    pub provenance: Provenance,
    /// The colour-ID part table.
    pub id_classes: [Part; 8],
    /// Both views.
    pub views: Vec<MetaView>,
}

/// Read a bake's `meta.json`.
///
/// # Errors
///
/// What does not hold: not format 3, a render under 384 px (rule 1), a view
/// missing or at the wrong pitch, a frame missing, a mark that is not a pair.
pub fn parse_meta(text: &str) -> Result<BakeMeta, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("meta.json: {e}"))?;
    if v.get("format").and_then(Value::as_u64) != Some(3) {
        return Err("meta.json: not format 3 (re-run the bake with this tree's Bake.cs)".into());
    }
    let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let model = s(&v, "model");
    if model.is_empty() {
        return Err("meta.json: no model".into());
    }
    let res = v
        .get("res")
        .and_then(Value::as_u64)
        .and_then(|r| usize::try_from(r).ok())
        .unwrap_or(0);
    if res < usize::from(RENDER_PX) {
        return Err(format!(
            "meta.json: a {res} px render; rule 1 reduces from {RENDER_PX} px or larger"
        ));
    }
    let pair = |o: Option<&Value>, what: &str| -> Result<(f64, f64), String> {
        match o.and_then(Value::as_array).map(Vec::as_slice) {
            Some([a, b, ..]) => match (a.as_f64(), b.as_f64()) {
                (Some(a), Some(b)) => Ok((a, b)),
                _ => Err(format!("meta.json: {what} is not a pair of numbers")),
            },
            _ => Err(format!("meta.json: {what} is not a pair")),
        }
    };
    let (w, h) = pair(v.get("size_cm"), "size_cm")?;
    let size_cm = (w.round().max(0.0) as u32, h.round().max(0.0) as u32);
    let p = v.get("provenance").cloned().unwrap_or(Value::Null);
    let provenance = Provenance {
        asset_pack: s(&p, "asset_pack"),
        asset_version: s(&p, "asset_version"),
        prefab: s(&p, "prefab"),
        clips: p
            .get("clips")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        bake_commit: s(&p, "bake_commit"),
        reduce_commit: String::new(),
    };
    let mut id_classes = DEFAULT_ID_CLASSES;
    if let Some(list) = v.get("id_classes").and_then(Value::as_array) {
        if list.len() != 8 {
            return Err("meta.json: id_classes names eight parts".into());
        }
        for (slot, word) in id_classes.iter_mut().zip(list) {
            *slot = word
                .as_str()
                .and_then(Part::named)
                .ok_or("meta.json: an id_classes entry is not paint/second/dark/accent/bright")?;
        }
    }
    let mut views = Vec::new();
    for view in View::ALL {
        let Some(mv) = v.get("views").and_then(Value::as_array).and_then(|vs| {
            vs.iter()
                .find(|x| x.get("view").and_then(Value::as_str) == Some(view.name()))
        }) else {
            return Err(format!("meta.json: no {} view", view.name()));
        };
        let pitch = mv
            .get("pitch_deg")
            .and_then(Value::as_f64)
            .unwrap_or(f64::NAN);
        if (pitch - f64::from(view.pitch_deg())).abs() > 0.01 {
            return Err(format!(
                "meta.json: the {} view is baked at {pitch}°, not {}°",
                view.name(),
                view.pitch_deg()
            ));
        }
        let span_cm = mv
            .get("span_cm")
            .and_then(Value::as_f64)
            .map_or(0, |s| s.round().max(0.0) as u32);
        if span_cm == 0 {
            return Err(format!("meta.json: the {} view has no span", view.name()));
        }
        let mut frames = Vec::new();
        for f in mv
            .get("frames")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let facing = Facing::named(&s(f, "facing"))
                .ok_or_else(|| format!("meta.json: facing {:?}", s(f, "facing")))?;
            let pose_word = s(f, "pose");
            let pose = POSES
                .iter()
                .position(|p| *p == pose_word)
                .ok_or_else(|| format!("meta.json: pose {pose_word:?}"))?;
            let (ax, ay) = pair(f.get("anchor"), "an anchor")?;
            let mut muzzles = Vec::new();
            for (name, xy) in f
                .get("muzzles")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                let (mx, my) = pair(Some(xy), "a muzzle")?;
                muzzles.push((name.clone(), (mx as f32, my as f32)));
            }
            frames.push(MetaFrame {
                facing,
                pose,
                anchor: (ax as f32, ay as f32),
                muzzles,
            });
        }
        for facing in Facing::ALL {
            for (pose, word) in POSES.iter().enumerate() {
                if !frames.iter().any(|f| f.facing == facing && f.pose == pose) {
                    return Err(format!(
                        "meta.json: no {} {} {word} frame",
                        view.name(),
                        facing.as_str()
                    ));
                }
            }
        }
        views.push(MetaView {
            view,
            span_cm,
            frames,
        });
    }
    Ok(BakeMeta {
        model,
        res,
        size_cm,
        provenance,
        id_classes,
        views,
    })
}

/// The file a render is in: `{view}_{facing}_{pose}_{pass}.png`, `pass` `id`
/// or `n`.
#[must_use]
pub fn render_name(view: View, facing: Facing, pose: usize, pass: &str) -> String {
    format!(
        "{}_{}_{}_{pass}.png",
        view.name(),
        facing.as_str(),
        POSES.get(pose).copied().unwrap_or("still")
    )
}

/// A render point at `res` as a pixel of an `n` frame.
fn scale_point((x, y): (f32, f32), res: usize, n: usize) -> (u8, u8) {
    let k = n as f32 / res.max(1) as f32;
    let px = |v: f32| {
        u8::try_from(((v * k).floor().max(0.0) as usize).min(n.saturating_sub(1))).unwrap_or(0)
    };
    (px(x), px(y))
}

/// **Reduce a whole bake** to its pack entry: for each view, facing and pose
/// the two renders `load(file name)` returns as RGBA (`res` by `res`),
/// classified once and reduced to every size of the view's chain; the marks
/// scaled from the render to each size; `reduce_commit` written into the
/// provenance.
///
/// # Errors
///
/// A render that does not load or is not `res` by `res`.
pub fn reduce_bake(
    meta: &BakeMeta,
    reduce_commit: &str,
    load: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<ModelBake, String> {
    let res = meta.res;
    // Pass one: every frame's materials, shade values and ink, and a shade
    // histogram a material over the whole model (the levels are the model's,
    // so a paint's ramp runs its full span whatever the kit's albedo makes of
    // its luminance).
    struct Classified {
        view: usize,
        facing: usize,
        pose: usize,
        codes: Vec<u8>,
        values: Vec<f32>,
        ink: Vec<bool>,
    }
    let mut classified = Vec::new();
    let mut hist = vec![[0u32; 256]; 16];
    for (vi, mv) in meta.views.iter().enumerate() {
        for (fi, &facing) in Facing::ALL.iter().enumerate() {
            for pose in 0..POSES.len() {
                if !mv
                    .frames
                    .iter()
                    .any(|f| f.facing == facing && f.pose == pose)
                {
                    continue;
                }
                let mut pass = |p: &str| -> Result<Option<Vec<u8>>, String> {
                    let name = render_name(mv.view, facing, pose, p);
                    match load(&name) {
                        Ok(rgba) if rgba.len() == res * res * 4 => Ok(Some(rgba)),
                        Ok(_) => Err(format!("{name}: not {res}x{res}")),
                        Err(e) if matches!(p, "id" | "n") => Err(e),
                        // The mask, lit and depth passes are optional: a bake
                        // from before them has none.
                        Err(_) => Ok(None),
                    }
                };
                let id = pass("id")?.ok_or("the colour-ID pass")?;
                let normal = pass("n")?.ok_or("the normal pass")?;
                let (mask, lit, depth) = (pass("m")?, pass("l")?, pass("d")?);
                let codes = classify(&id, &normal, mask.as_deref(), &meta.id_classes);
                let depth_m = depth.as_deref().map(depth_metres);
                let (values, ink) = shade_values(&normal, lit.as_deref(), depth_m.as_deref(), res);
                for (i, &c) in codes.iter().enumerate() {
                    if c != 0 {
                        let v = values.get(i).copied().unwrap_or(0.0);
                        let bin = (v / SHADE_SPAN * 255.0).clamp(0.0, 255.0) as usize;
                        hist[usize::from(c)][bin] += 1;
                    }
                }
                classified.push(Classified {
                    view: vi,
                    facing: fi,
                    pose,
                    codes,
                    values,
                    ink,
                });
            }
        }
    }
    let levels: Vec<(f32, f32)> = hist.iter().map(percentile_levels).collect();
    // Pass two: the shade quantised between its material's levels, and the
    // frame reduced to every size of its view's chain.
    let mut chains: Vec<Chain> = meta
        .views
        .iter()
        .map(|mv| Chain {
            view: mv.view,
            span_cm: mv.span_cm,
            sizes: mv
                .view
                .sizes()
                .iter()
                .map(|&size| SizedFrames {
                    size,
                    frames: vec![BakedFrame::default(); FRAMES],
                })
                .collect(),
        })
        .collect();
    for f in &classified {
        let mv = &meta.views[f.view];
        let facing = Facing::ALL[f.facing];
        let Some(mf) = mv
            .frames
            .iter()
            .find(|m| m.facing == facing && m.pose == f.pose)
        else {
            continue;
        };
        let hi: Vec<u8> = f
            .codes
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                if c == 0 {
                    0
                } else if f.ink.get(i).copied().unwrap_or(false) {
                    c
                } else {
                    let v = f.values.get(i).copied().unwrap_or(0.0);
                    c | (quantise(v, levels[usize::from(c)]) << 4)
                }
            })
            .collect();
        for s in &mut chains[f.view].sizes {
            let n = usize::from(s.size);
            let grid = reduce(&hi, res, n);
            let muzzles = mf
                .muzzles
                .iter()
                .map(|(name, xy)| (name.clone(), scale_point(*xy, res, n)))
                .collect();
            if let Some(slot) = s.frames.get_mut(f.facing * POSES.len() + f.pose) {
                *slot = BakedFrame::from_grid(&grid, n, scale_point(mf.anchor, res, n), muzzles);
            }
        }
    }
    Ok(ModelBake {
        model: meta.model.clone(),
        provenance: Provenance {
            reduce_commit: reduce_commit.to_string(),
            ..meta.provenance.clone()
        },
        size_cm: meta.size_cm,
        chains,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::{FP_SIZES, ISO_SIZES, Pack, check_model};

    const RES: usize = 384;

    /// **A synthetic bake** standing in for the owner's Unity render (the
    /// licensed packs never leave the owner's machine): a box-bodied tank
    /// with a turret and a barrel at each view's pitch, its colour-ID and
    /// normal passes as RGBA, every facing and pose, and the `meta.json`
    /// `Bake.cs` writes. The body is green paint (lit top, shaded sides),
    /// the tracks black, a lamp pale; the barrel is three render pixels thin.
    fn render(view: View, facing: usize, pose: usize) -> (Vec<u8>, Vec<u8>) {
        let mut id = vec![0u8; RES * RES * 4];
        let mut n = vec![0u8; RES * RES * 4];
        let iso = view == View::Iso;
        let wide = matches!(facing, 2 | 6); // a side on: the hull's length
        let (hw, top) = if wide { (150, 200) } else { (80, 200) };
        let roof = if iso { 60 } else { 8 };
        let sink = if pose == 5 { 20 } else { 0 };
        let bob = if (1..5).contains(&pose) { pose % 2 } else { 0 };
        let mut put = |x: usize, y: usize, rgb: [u8; 3], normal: [u8; 3]| {
            let i = (y * RES + x) * 4;
            id[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            n[i..i + 4].copy_from_slice(&[normal[0], normal[1], normal[2], 255]);
        };
        let up = [128, 255, 128];
        let side = [128, 128, 20];
        for y in top + sink + bob..290 + sink {
            for x in 192 - hw..192 + hw {
                let is_roof = y < top + roof + sink + bob;
                let tracks = y >= 260 + sink;
                let rgb = if tracks { [0, 0, 0] } else { [0, 255, 0] };
                put(x, y, rgb, if is_roof { up } else { side });
            }
        }
        // The turret and a barrel three pixels thin, and a pale lamp.
        for y in 160 + sink..top + sink {
            for x in 150..234 {
                put(x, y, [0, 255, 0], up);
            }
        }
        for y in 175 + sink..178 + sink {
            for x in 234..330 {
                put(x, y, [0, 0, 0], side);
            }
        }
        for y in 230..236 {
            for x in 192 - hw + 6..192 - hw + 14 {
                put(x, y, [255, 255, 255], side);
            }
        }
        (id, n)
    }

    fn meta_json() -> String {
        let frames = |view: View| -> Vec<Value> {
            Facing::ALL
                .iter()
                .flat_map(|f| {
                    POSES.iter().map(move |p| {
                        serde_json::json!({
                            "facing": f.as_str(),
                            "pose": p,
                            "anchor": [192.0, if view == View::Iso { 275.5 } else { 289.4 }],
                            "muzzles": { "cannon": [329.0, 176.0] }
                        })
                    })
                })
                .collect()
        };
        serde_json::json!({
            "format": 3,
            "model": "mc.st.tank_lvl2",
            "res": RES,
            "size_cm": [700, 250],
            "provenance": {
                "asset_pack": "Synthetic Test Tank",
                "asset_version": "0",
                "prefab": "synthetic",
                "clips": ["none"],
                "bake_commit": "test"
            },
            "views": [
                { "view": "fp", "pitch_deg": 3.0, "span_cm": 1000, "frames": frames(View::Fp) },
                { "view": "iso", "pitch_deg": 28.0, "span_cm": 1200, "frames": frames(View::Iso) }
            ]
        })
        .to_string()
    }

    /// The synthetic bake reduced once for the tests that read it.
    fn reduced() -> ModelBake {
        static REDUCED: std::sync::OnceLock<ModelBake> = std::sync::OnceLock::new();
        REDUCED.get_or_init(reduce_synthetic).clone()
    }

    fn reduce_synthetic() -> ModelBake {
        let meta = parse_meta(&meta_json()).expect("the synthetic meta parses");
        let mut load = |name: &str| -> Result<Vec<u8>, String> {
            let mut parts = name.trim_end_matches(".png").split('_');
            let view = View::named(parts.next().unwrap_or("")).ok_or("view")?;
            let rest: Vec<&str> = parts.collect();
            let pass = rest.last().copied().unwrap_or("");
            let pose_word = rest.get(rest.len() - 2).copied().unwrap_or("");
            let facing_word = rest[..rest.len() - 2].join("_");
            let facing = Facing::ALL
                .iter()
                .position(|f| f.as_str() == facing_word)
                .ok_or("facing")?;
            let pose = POSES.iter().position(|p| *p == pose_word).ok_or("pose")?;
            if pass != "id" && pass != "n" {
                return Err("the synthetic bake has the colour-ID and normal passes only".into());
            }
            let (id, n) = render(view, facing, pose);
            Ok(if pass == "id" { id } else { n })
        };
        reduce_bake(&meta, "reducecommit", &mut load).expect("the synthetic bake reduces")
    }

    #[test]
    fn reduce_votes_every_material_code() {
        // A 4x4 render of steel-light (8) with one lamp (11) reduces to a
        // 2x2 of 8 with the lamp's cell 11: codes past six are not pale.
        let mut hi = vec![8u8; 16];
        hi[0] = 11;
        hi[1] = 11;
        hi[4] = 11;
        assert_eq!(reduce(&hi, 4, 2), vec![11, 8, 8, 8]);
    }

    #[test]
    fn classify_names_the_family_and_keeps_the_parts() {
        let px = |rgb: [u8; 3], a: u8| vec![rgb[0], rgb[1], rgb[2], a];
        let up = px([128, 255, 128], 255);
        let codes = |id: Vec<u8>| classify(&id, &up, None, &DEFAULT_ID_CLASSES)[0];
        assert_eq!(codes(px([0, 255, 0], 255)), 2, "paint: the team's base");
        assert_eq!(codes(px([0, 0, 0], 255)), 4, "dark metal: black");
        assert_eq!(codes(px([0, 0, 255], 255)), 5, "secondary: the steel base");
        assert_eq!(codes(px([255, 0, 0], 255)), 3, "accent: team-light");
        assert_eq!(codes(px([255, 255, 255], 255)), 6, "bright: pale");
        assert_eq!(codes(px([0, 255, 0], 0)), 0, "transparent");
        // The mask pass: a rubber-flagged pixel is rubber whatever its part;
        // a lit pixel is a lamp whatever its part.
        let masked = |id: Vec<u8>, m: Vec<u8>| classify(&id, &up, Some(&m), &DEFAULT_ID_CLASSES)[0];
        assert_eq!(
            masked(px([0, 255, 0], 255), px([0, 255, 0], 255)),
            9,
            "rubber"
        );
        assert_eq!(
            masked(px([0, 0, 255], 255), px([0, 0, 0], 255)),
            5,
            "unflagged: steel"
        );
        assert_eq!(
            masked(px([0, 255, 0], 255), px([0, 0, 255], 255)),
            11,
            "a lamp"
        );
    }

    #[test]
    fn the_shade_plane_lights_levels_and_inks() {
        let px = |rgb: [u8; 3], a: u8| vec![rgb[0], rgb[1], rgb[2], a];
        let (up, down) = (px([128, 255, 128], 255), px([128, 0, 128], 255));
        let v = shade_from_normal(&[up.clone(), down.clone()].concat());
        assert!(
            v[0] > 0.7 && v[1] < 0.2,
            "a face up is lit, a face down is dark"
        );
        assert_eq!(quantise(0.0, (0.0, 1.0)), 1);
        assert_eq!(quantise(1.0, (0.0, 1.0)), 15);
        assert_eq!(quantise(2.0, (0.0, 1.0)), 15, "clamped");
        assert_eq!(
            percentile_levels(&[0; 256]),
            (0.0, 1.0),
            "nothing: a unit span"
        );
        // The depth pass: a step of a metre between neighbours is ink, and the
        // lower pixel beside a nearer one is occluded.
        let res = 16;
        let mut depth = vec![f32::INFINITY; res * res];
        let mut normal = Vec::new();
        for i in 0..res * res {
            let x = i % res;
            depth[i] = if x < 8 { 200.0 } else { 201.0 };
            normal.extend_from_slice(&up);
        }
        let inked = ink(&normal, Some(&depth), res);
        assert!(
            inked[7] && inked[8] && !inked[3] && !inked[12],
            "the step is ink"
        );
        let ao = occlusion(&depth, res);
        assert!(
            ao[8 + res * 8] < 1.0 && (ao[1 + res * 8] - 1.0).abs() < 1e-6,
            "the far side is occluded"
        );
        // A cell takes the mean shade of its material, or the ink when four
        // tenths of it are ink.
        assert_eq!(
            reduce(
                &[5 | (4 << 4), 5 | (6 << 4), 5 | (8 << 4), 5 | (10 << 4)],
                2,
                1
            ),
            vec![5 | (7 << 4)]
        );
        assert_eq!(
            reduce(&[5, 5, 5 | (8 << 4), 5 | (8 << 4)], 2, 1),
            vec![5],
            "half the cell on the ink line"
        );
        assert_eq!(
            reduce(&[5, 5 | (8 << 4), 5 | (8 << 4), 5 | (8 << 4)], 2, 1),
            vec![5 | (8 << 4)]
        );
    }

    #[test]
    fn reduce_keeps_masses_and_thin_parts() {
        // A 3 px line across a 384 render survives to 96 (4 px cells, 75 %)
        // and to 48 (8 px cells, 37 %) but not to 24 (16 px cells, 19 %).
        let mut hi = vec![0u8; RES * RES];
        for x in 0..RES {
            for y in 100..103 {
                hi[y * RES + x] = 4;
            }
        }
        let rows = |n: usize| -> usize {
            let g = reduce(&hi, RES, n);
            (0..n)
                .filter(|r| g[r * n..(r + 1) * n].iter().all(|&c| c == 4))
                .count()
        };
        assert_eq!(rows(96), 1);
        assert_eq!(rows(48), 1);
        assert_eq!(rows(24), 0);
        // Fractional cells (384/80 = 4.8) tile the render with nothing lost.
        let full = vec![2u8; RES * RES];
        for n in [80, 56, 40] {
            assert!(reduce(&full, RES, n).iter().all(|&c| c == 2), "{n}");
        }
        // The most common material wins a cell.
        let mut mixed = vec![1u8; RES * RES];
        for v in mixed.iter_mut().step_by(3) {
            *v = 6;
        }
        assert!(reduce(&mixed, RES, 8).iter().all(|&c| c == 1));
    }

    /// The synthetic bake reduces to a complete entry: both chains at every
    /// size, from the 384 render (an 8 px frame is not the 12's reduced: it
    /// matches a direct reduction of the render), the marks inside, the
    /// provenance carried, and it survives the pack round trip.
    #[test]
    fn a_bake_reduces_to_both_chains_from_the_large_render() {
        let e = reduced();
        assert_eq!(e.model, "mc.st.tank_lvl2");
        assert_eq!(e.provenance.reduce_commit, "reducecommit");
        assert_eq!(e.provenance.asset_pack, "Synthetic Test Tank");
        let fp = e.chain(View::Fp).unwrap();
        let iso = e.chain(View::Iso).unwrap();
        assert_eq!(
            fp.sizes.iter().map(|s| s.size).collect::<Vec<_>>(),
            FP_SIZES
        );
        assert_eq!(
            iso.sizes.iter().map(|s| s.size).collect::<Vec<_>>(),
            ISO_SIZES
        );
        // Rule 1: the 8 px frame is the render reduced to 8, not the 12 px
        // frame reduced again.
        let (id, n) = render(View::Fp, 0, 0);
        let hi = classify(&id, &n, None, &DEFAULT_ID_CLASSES);
        let direct = BakedFrame::from_grid(&reduce(&hi, RES, 8), 8, (4, 6), Vec::new());
        let eight = fp.sizes[8].frame(Facing::Front, 0).unwrap();
        assert_eq!(
            eight.pixels.iter().map(|c| c & 0x0f).collect::<Vec<_>>(),
            direct.pixels,
            "the materials; the shade plane rides above them"
        );
        // The marks scale with the size (the 192 px frame is the render at
        // 2:1); the iso view shows more roof.
        let biggest = fp.sizes[0].frame(Facing::Front, 0).unwrap();
        assert_eq!(biggest.anchor, (96, 144));
        assert_eq!(biggest.muzzles, vec![("cannon".to_string(), (164, 88))]);
        let big = fp.sizes[1].frame(Facing::Front, 0).unwrap();
        assert_eq!(big.anchor, (48, 72));
        assert_eq!(big.muzzles, vec![("cannon".to_string(), (82, 44))]);
        let lit = |f: &BakedFrame| f.pixels.iter().filter(|&&c| c != 0 && c >> 4 >= 11).count();
        let iso_big = iso.sizes[0].frame(Facing::Front, 0).unwrap();
        assert!(lit(iso_big) > lit(big) * 2, "the 28° bake shows the roof");
        // The wreck is not the still.
        assert_ne!(
            fp.sizes[0].frame(Facing::Front, 5),
            fp.sizes[0].frame(Facing::Front, 0)
        );
        let mut pack = Pack::default();
        pack.put(e.clone());
        assert_eq!(Pack::decode(&pack.encode()).unwrap().entries, vec![e]);
    }

    /// The synthetic entry holds the matrix check against the tracked
    /// tank's look, which names its model.
    #[test]
    fn the_synthetic_entry_holds_the_frame_rules() {
        assert_eq!(
            check_model(&reduced(), crate::look::shipped()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_meta_refuses_a_small_render_or_a_wrong_pitch() {
        let ok = meta_json();
        assert!(parse_meta(&ok).is_ok());
        assert!(
            parse_meta(&ok.replace("\"res\":384", "\"res\":96"))
                .unwrap_err()
                .contains("rule 1")
        );
        assert!(parse_meta(&ok.replace("\"pitch_deg\":3.0", "\"pitch_deg\":8.0")).is_err());
        assert!(parse_meta(&ok.replace("\"format\":3", "\"format\":2")).is_err());
        let one_short = ok.replacen("\"pose\":\"wreck\"", "\"pose\":\"still\"", 1);
        assert!(parse_meta(&one_short).unwrap_err().contains("wreck"));
    }
}
