//! Embedded SPA serving: `/`, `/assets/*`, and the SPA deep-link fallback.
//!
//! Release builds embed `web/dist` (the Vite build output) into the binary
//! via `rust-embed`; debug builds read the same folder from disk at runtime
//! (rust-embed's default), so frontend iteration needs no server rebuild —
//! though during frontend dev you normally run `vite dev`, which proxies
//! `/api` + `/ws` here instead.
//!
//! Cache policy (design 02 §6.1): Vite content-hashes everything under
//! `assets/`, so those get `public, max-age=31536000, immutable`;
//! `index.html` (and any other non-hashed file) gets `no-cache` so a deploy
//! is picked up on the next load.

use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The Vite build output. The folder is guaranteed to exist by `build.rs`;
/// when the SPA has not been built it is simply empty and the routes 404.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebDist;

/// Cache-Control for content-hashed assets.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// Cache-Control for everything else (index.html above all).
const NO_CACHE: &str = "no-cache";

/// Router fallback: serves the SPA and its assets for anything no explicit
/// route claimed.
///
/// - `/` → `index.html`
/// - `/assets/…` and any other embedded file → that file
/// - unknown non-API path → `index.html` (SPA deep links)
/// - unknown path under `/api/` (or `/ws`, `/healthz`) → plain 404, never
///   the SPA
pub(crate) async fn static_handler(method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = WebDist::get(path) {
        return file_response(path, file);
    }
    // SPA deep-link fallback — but never shadow the API namespace.
    if path.starts_with("api/") || path == "ws" || path == "healthz" {
        return StatusCode::NOT_FOUND.into_response();
    }
    match WebDist::get("index.html") {
        Some(file) => file_response("index.html", file),
        None => (
            StatusCode::NOT_FOUND,
            "web client not built (run `npm run build` in web/)",
        )
            .into_response(),
    }
}

fn file_response(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    let cache = if path.starts_with("assets/") {
        IMMUTABLE
    } else {
        NO_CACHE
    };
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(mime_for(path)),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        file.data,
    )
        .into_response()
}

/// Content type by extension — the handful Vite actually emits (a tiny
/// static map beats pulling in mime_guess for six file kinds).
fn mime_for(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map" | "webmanifest") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        // Doc 21 cut G.3b: the cockpit picture's module, which a browser
        // streams and compiles only under this type.
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_map_covers_vite_output() {
        assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            mime_for("assets/index-DmrAefkR.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            mime_for("assets/index-Bl9qK872.css"),
            "text/css; charset=utf-8"
        );
        assert_eq!(mime_for("assets/index.js.map"), "application/json");
        assert_eq!(mime_for("favicon.ico"), "image/x-icon");
        assert_eq!(
            mime_for("assets/mush_pixel-C3d4e5f6.wasm"),
            "application/wasm"
        );
        assert_eq!(mime_for("noextension"), "application/octet-stream");
    }
}
