//! Guards the SPA embed: `rust-embed` expands over `../../web/dist`.
//!
//! - The folder is created if missing so the `#[derive(RustEmbed)]`
//!   expansion itself never fails on a fresh clone.
//! - **Release builds require a built SPA**: `web/dist/index.html` missing
//!   means someone is about to ship a server whose web client 404s — that
//!   is a compile error with the fix in the message, not a runtime
//!   surprise on the production box.
//! - Debug builds only warn (they read `web/dist` from disk at runtime, so
//!   building the SPA later works without recompiling; `vite dev` proxying
//!   `/api`/`/ws` needs no dist at all).
//! - `rerun-if-changed` on the dist folder re-runs this script (and
//!   therefore re-embeds) when `npm run build` writes a new bundle, so a
//!   release binary can never carry a stale SPA silently.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    // Best-effort: a read-only checkout still builds if the folder exists.
    let _ = std::fs::create_dir_all(&dist);
    // The folder mtime changes whenever Vite replaces the bundle (new
    // hashed filenames), which re-runs this script and recompiles the
    // crate — refreshing the release embed. index.html is tracked
    // explicitly too (its content changes without a rename).
    println!("cargo:rerun-if-changed={}", dist.display());
    println!(
        "cargo:rerun-if-changed={}",
        dist.join("index.html").display()
    );

    if dist.join("index.html").exists() {
        return;
    }
    let profile = std::env::var("PROFILE").unwrap_or_default();
    if profile == "release" {
        panic!(
            "\n\nweb client not built: {} does not exist.\n\
             Release builds embed the SPA into the binary; build it first:\n\
             \n    cd web && npm install && npm run build\n\n\
             (Debug builds serve web/dist from disk and only warn.)\n",
            dist.join("index.html").display()
        );
    }
    println!(
        "cargo:warning=web/dist/index.html missing: the web client will 404 \
         until you run `npm run build` in web/ (debug builds pick it up \
         without a rebuild)"
    );
}
