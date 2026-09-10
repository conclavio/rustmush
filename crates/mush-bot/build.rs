use std::{env, fs, path::PathBuf};

fn main() {
    // Ship the game's help with the binary; no checkout needed at runtime.
    let help = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../docs/help");
    println!("cargo:rerun-if-changed={}", help.display());
    let mut files: Vec<_> = fs::read_dir(help)
        .expect("RustMUSH help directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    let mut source = String::from("pub const BUNDLED_HELP: &[(&str, &str)] = &[\n");
    for path in files {
        let name = format!("help {}", path.file_stem().unwrap().to_string_lossy());
        let body = fs::read_to_string(path).unwrap();
        source.push_str(&format!("({name:?}, {body:?}),\n"));
    }
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("help.rs"),
        source,
    )
    .unwrap();
}
