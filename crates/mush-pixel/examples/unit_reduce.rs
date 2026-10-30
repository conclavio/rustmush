//! **Reduce one model's bake into the catalogue** (doc 22 D-22-10, cut U.6;
//! doc 31 D-31-2, cut UD.1): reads the 384 px renders and `meta.json` that
//! `scripts/unit-bake/unity/Editor/Bake.cs` wrote for one model, reduces every
//! size of both chains from them (`mush_pixel::bake::reduce`, the only
//! resampler), writes the model's own content-named file under `models/` —
//! removing its older file — and its entry in `models/index.json`
//! (`mush_pixel::model::write_model`), and prints the entry's size by chain,
//! the file's size against rule 7's per-model ceiling, its SHA-256 and the
//! matrix check against the looks that name the model. With `--preview`, a
//! PNG of every size's still frames, both views, for an eye check.
//!
//! ```text
//! cargo run --release -p mush-pixel --example unit_reduce -- <bake_out/model> //!     [--models crates/mush-pixel/models] [--reduce-commit <sha>] [--preview <png>]
//! ```
//!
//! `scripts/unit-bake/bake.sh` runs it after each model's bake.

use std::path::PathBuf;

use mush_pixel::bake::{self, Pack, View, reduce};
use mush_pixel::look::{self, Facing};
use mush_pixel::{model, png};

const USAGE: &str = "usage: unit_reduce <bake_out/model> [--models <dir>] [--reduce-commit <sha>] [--preview <png>]";

fn main() {
    if let Err(e) = run() {
        eprintln!("unit_reduce: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut dir: Option<PathBuf> = None;
    let mut models_dir = PathBuf::from("crates/mush-pixel/models");
    let mut commit = String::new();
    let mut preview: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--models" => models_dir = args.next().ok_or(USAGE)?.into(),
            "--reduce-commit" => commit = args.next().ok_or(USAGE)?,
            "--preview" => preview = Some(args.next().ok_or(USAGE)?.into()),
            _ if dir.is_none() && !a.starts_with("--") => dir = Some(a.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    let dir = dir.ok_or(USAGE)?;
    let meta_path = dir.join("meta.json");
    let text =
        std::fs::read_to_string(&meta_path).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    let meta = reduce::parse_meta(&text)?;
    if !look::shipped()
        .iter()
        .any(|l| l.model.as_deref() == Some(meta.model.as_str()))
    {
        eprintln!(
            "unit_reduce: {}: no shipped look names this model yet (add `\"model\"` to one)",
            meta.model
        );
    }
    let res = meta.res;
    let mut load = |name: &str| -> Result<Vec<u8>, String> {
        let path = dir.join(name);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (w, h, rgba) = png::decode_rgba(&bytes).map_err(|e| format!("{name}: {e}"))?;
        if (w, h) != (res, res) {
            return Err(format!("{name}: {w}x{h}, not {res}x{res}"));
        }
        Ok(rgba)
    };
    let entry = reduce::reduce_bake(&meta, &commit, &mut load)?;

    println!(
        "{}: {} x {} cm, from {} {} ({}), clips {:?}",
        entry.model,
        entry.size_cm.0,
        entry.size_cm.1,
        entry.provenance.asset_pack,
        entry.provenance.asset_version,
        entry.provenance.prefab,
        entry.provenance.clips
    );
    for chain in &entry.chains {
        let alone = Pack {
            entries: vec![bake::ModelBake {
                chains: vec![chain.clone()],
                ..entry.clone()
            }],
        };
        println!(
            "  {} chain: span {} cm, {} bytes packed",
            chain.view.name(),
            chain.span_cm,
            alone.encode().len()
        );
    }
    if let Some(path) = &preview {
        std::fs::write(path, preview_png(&entry))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        println!("  preview: {}", path.display());
    }

    let bad = bake::check_model(&entry, look::shipped());
    let written = model::write_model(&models_dir, entry)?;
    println!(
        "model {}: {} bytes ({:.0} KiB; rule 7's per-model ceiling is {} KiB), sha-256 {}",
        models_dir.join(&written.file).display(),
        written.bytes,
        written.bytes as f64 / 1024.0,
        bake::BUDGET_BYTES / 1024,
        written.sha256
    );
    if bad.is_empty() {
        println!("matrix check: holds");
    } else {
        println!("matrix check: {} findings", bad.len());
        for b in bad.iter().take(20) {
            println!("  {b}");
        }
    }
    Ok(())
}

/// Every size's still front, right, rear and left, the `fp` chain above the
/// `iso`, a 104 px cell each, in the red team's colours over a dark ground.
fn preview_png(entry: &bake::ModelBake) -> Vec<u8> {
    let facings = [Facing::Front, Facing::Right, Facing::Rear, Facing::Left];
    let cell = 104usize;
    let sizes = bake::FP_SIZES.len();
    let (w, h) = (cell * facings.len() * 2, cell * sizes);
    let mut rgba = [40u8, 44, 40, 255].repeat(w * h);
    for (vi, view) in View::ALL.iter().enumerate() {
        let Some(chain) = entry.chain(*view) else {
            continue;
        };
        for (si, sized) in chain.sizes.iter().enumerate() {
            for (fi, facing) in facings.iter().enumerate() {
                let Some(f) = sized.frame(*facing, 0) else {
                    continue;
                };
                let (ox, oy) = ((vi * facings.len() + fi) * cell + 4, si * cell + 4);
                let n = usize::from(sized.size);
                for y in 0..n {
                    for x in 0..n {
                        let rgb = match bake::material_from_code(f.code(x as i64, y as i64)) {
                            Some(m) => colour(m),
                            None if (x, y)
                                == (usize::from(f.anchor.0), usize::from(f.anchor.1)) =>
                            {
                                [255, 255, 0]
                            }
                            None => continue,
                        };
                        let i = ((oy + y) * w + ox + x) * 4;
                        if let Some(p) = rgba.get_mut(i..i + 4) {
                            p.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                        }
                    }
                }
            }
        }
    }
    png::encode_rgba(w, h, &rgba)
}

fn colour(m: look::Material) -> [u8; 3] {
    match m {
        look::Material::TeamDark => [120, 36, 32],
        look::Material::Team => [178, 54, 48],
        look::Material::TeamLight => [214, 98, 84],
        look::Material::Black => [20, 22, 26],
        look::Material::Steel => [118, 124, 132],
        look::Material::Pale => [216, 216, 198],
    }
}
