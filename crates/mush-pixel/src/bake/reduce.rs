//! **The reducer** (doc 22 D-22-10, rules 1 and 2): one model's bake — the
//! 384 px colour-ID and normal renders `Bake.cs` writes for both views, eight
//! facings and six poses, and its `meta.json` — reduced to the model's pack
//! entry (doc 31 D-31-1). It is the only resampler: every size of both chains is reduced from
//! the large render, never from a smaller size, as masses (a cell is filled
//! when 30 % of its render pixels are, so thin parts survive; its material is
//! the most common among them).
//!
//! The material of a render pixel is its part (the colour-ID pass: three bits
//! of red, green and blue name the part, [`Part`]) lit by a light fixed to the
//! viewer, upper left and toward the eye (the normal pass): the main paint into
//! the three team tones by light, secondary panels steel or team-dark, dark
//! metal black, accents team-light, lamps and glass pale — the rule the
//! sprite-vs-voxel PoC's `reduce.py` used, which this replaces.
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
    /// Secondary panels: steel lit, team-dark in shadow.
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

/// Classify one render: each pixel's pack code (`0` nothing, `1..=6` the
/// materials) from its colour-ID and world-normal pixels (RGBA, the same
/// size), with the part table `classes`.
#[must_use]
pub fn classify(id_rgba: &[u8], normal_rgba: &[u8], classes: &[Part; 8]) -> Vec<u8> {
    // Viewer-relative light: upper left, toward the viewer (the camera looks
    // along +Z), normalised.
    let (lx, ly, lz) = (-0.4f32, 0.8f32, -0.45f32);
    let len = (lx * lx + ly * ly + lz * lz).sqrt();
    let light = [lx / len, ly / len, lz / len];
    id_rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(normal_rgba.as_chunks::<4>().0)
        .map(|(&[r, g, b, a], &[nx, ny, nz, _])| {
            if a <= 127 {
                return 0;
            }
            let bits = usize::from(r > 127) * 4 + usize::from(g > 127) * 2 + usize::from(b > 127);
            let unit = |v: u8| f32::from(v) / 255.0 * 2.0 - 1.0;
            let lit =
                (unit(nx) * light[0] + unit(ny) * light[1] + unit(nz) * light[2]).clamp(0.0, 1.0);
            match classes.get(bits).copied().unwrap_or(Part::Dark) {
                Part::Paint if lit > 0.72 => 3,
                Part::Paint if lit > 0.35 => 2,
                Part::Paint => 1,
                Part::Second if lit > 0.45 => 5,
                Part::Second => 1,
                Part::Dark => 4,
                Part::Accent => 3,
                Part::Bright => 6,
            }
        })
        .collect()
}

/// **Reduce** a `res` by `res` grid of codes to `n` by `n` as masses: a cell
/// covers the render pixels between its edges (`c·res/n` to `(c+1)·res/n`),
/// is drawn when at least three tenths of them are, and takes the most common
/// material among them (the lower code on a tie).
#[must_use]
pub fn reduce(hi: &[u8], res: usize, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n * n];
    if n == 0 || res < n {
        return out;
    }
    for (r, row) in out.chunks_exact_mut(n).enumerate() {
        let (y0, y1) = (r * res / n, (r + 1) * res / n);
        for (c, cell) in row.iter_mut().enumerate() {
            let (x0, x1) = (c * res / n, (c + 1) * res / n);
            let mut counts = [0usize; 7];
            for y in y0..y1 {
                for x in x0..x1 {
                    let code = usize::from(hi.get(y * res + x).copied().unwrap_or(0)).min(6);
                    if let Some(k) = counts.get_mut(code) {
                        *k += 1;
                    }
                }
            }
            let total = (y1 - y0) * (x1 - x0);
            let filled: usize = counts.iter().skip(1).sum();
            if filled * 10 < total * COVERAGE_TENTHS || filled == 0 {
                continue;
            }
            let mut best = (0usize, 0u8);
            for (code, &k) in counts.iter().enumerate().skip(1) {
                if k > best.0 {
                    best = (k, u8::try_from(code).unwrap_or(0));
                }
            }
            *cell = best.1;
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
    let mut chains = Vec::new();
    for mv in &meta.views {
        let mut sizes: Vec<SizedFrames> = mv
            .view
            .sizes()
            .iter()
            .map(|&size| SizedFrames {
                size,
                frames: vec![BakedFrame::default(); FRAMES],
            })
            .collect();
        for (fi, &facing) in Facing::ALL.iter().enumerate() {
            for pose in 0..POSES.len() {
                let Some(mf) = mv
                    .frames
                    .iter()
                    .find(|f| f.facing == facing && f.pose == pose)
                else {
                    continue;
                };
                let mut pass = |p: &str| -> Result<Vec<u8>, String> {
                    let name = render_name(mv.view, facing, pose, p);
                    let rgba = load(&name)?;
                    if rgba.len() != res * res * 4 {
                        return Err(format!("{name}: not {res}x{res}"));
                    }
                    Ok(rgba)
                };
                let (id, normal) = (pass("id")?, pass("n")?);
                let hi = classify(&id, &normal, &meta.id_classes);
                for s in &mut sizes {
                    let n = usize::from(s.size);
                    let grid = reduce(&hi, res, n);
                    let muzzles = mf
                        .muzzles
                        .iter()
                        .map(|(name, xy)| (name.clone(), scale_point(*xy, res, n)))
                        .collect();
                    if let Some(slot) = s.frames.get_mut(fi * POSES.len() + pose) {
                        *slot = BakedFrame::from_grid(
                            &grid,
                            n,
                            scale_point(mf.anchor, res, n),
                            muzzles,
                        );
                    }
                }
            }
        }
        chains.push(Chain {
            view: mv.view,
            span_cm: mv.span_cm,
            sizes,
        });
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
            let (id, n) = render(view, facing, pose);
            Ok(if pass == "id" { id } else { n })
        };
        reduce_bake(&meta, "reducecommit", &mut load).expect("the synthetic bake reduces")
    }

    #[test]
    fn classify_lights_the_paint_and_keeps_the_parts() {
        let px = |rgb: [u8; 3], a: u8| vec![rgb[0], rgb[1], rgb[2], a];
        let up = px([128, 255, 128], 255);
        let down = px([128, 0, 128], 255);
        let codes = |id: Vec<u8>, n: &Vec<u8>| classify(&id, n, &DEFAULT_ID_CLASSES)[0];
        assert_eq!(codes(px([0, 255, 0], 255), &up), 3, "lit paint: team-light");
        assert_eq!(
            codes(px([0, 255, 0], 255), &down),
            1,
            "paint in shadow: team-dark"
        );
        assert_eq!(codes(px([0, 0, 0], 255), &up), 4, "dark metal: black");
        assert_eq!(codes(px([0, 0, 255], 255), &up), 5, "lit secondary: steel");
        assert_eq!(codes(px([255, 0, 0], 255), &up), 3, "accent");
        assert_eq!(codes(px([255, 255, 255], 255), &up), 6, "bright: pale");
        assert_eq!(codes(px([0, 255, 0], 0), &up), 0, "transparent");
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
        let hi = classify(&id, &n, &DEFAULT_ID_CLASSES);
        let direct = BakedFrame::from_grid(&reduce(&hi, RES, 8), 8, (4, 6), Vec::new());
        let eight = fp.sizes[7].frame(Facing::Front, 0).unwrap();
        assert_eq!(eight.pixels, direct.pixels);
        // The marks scale with the size; the iso view shows more roof.
        let big = fp.sizes[0].frame(Facing::Front, 0).unwrap();
        assert_eq!(big.anchor, (48, 72));
        assert_eq!(big.muzzles, vec![("cannon".to_string(), (82, 44))]);
        let lit = |f: &BakedFrame| f.pixels.iter().filter(|&&c| c == 3).count();
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
