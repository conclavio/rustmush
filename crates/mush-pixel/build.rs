//! Doc 31 cut UD.1 (D-31-2, D-31-3): list `models/` — one content-named
//! `.mupk` file a baked model, one `.sheet.json` fallback sheet a model —
//! into `$OUT_DIR/models.rs`, so native builds embed every shipped model
//! without a hand-kept list of file names that change with every bake. The
//! WebAssembly build (no `encode` feature) embeds the sheets and none of the
//! model files: the page fetches those.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let dir = manifest.join("models");
    println!("cargo:rerun-if-changed=models");
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut out = String::new();
    let list = |out: &mut String, name: &str, ty: &str, mac: &str, suffix: &str| {
        let _ = writeln!(out, "pub(crate) const {name}: &[(&str, {ty})] = &[");
        for (file, path) in files.iter().filter(|(f, _)| f.ends_with(suffix)) {
            let _ = writeln!(
                out,
                "    ({file:?}, {mac}!({:?})),",
                path.display().to_string()
            );
        }
        let _ = writeln!(out, "];");
    };
    out.push_str("#[cfg(feature = \"encode\")]\n");
    list(
        &mut out,
        "EMBEDDED_MODELS",
        "&[u8]",
        "include_bytes",
        ".mupk",
    );
    out.push_str(
        "#[cfg(not(feature = \"encode\"))]\npub(crate) const EMBEDDED_MODELS: &[(&str, &[u8])] = &[];\n",
    );
    list(
        &mut out,
        "MODEL_SHEETS",
        "&str",
        "include_str",
        ".sheet.json",
    );
    let dest = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it")).join("models.rs");
    std::fs::write(dest, out).expect("OUT_DIR is writable");
}
